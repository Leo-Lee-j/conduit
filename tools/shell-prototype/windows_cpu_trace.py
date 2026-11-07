"""Windows CPU context for isolated probes; observations never certify a gate.

Counter definitions follow the Win32 GetSystemTimes and GetProcessTimes docs.
GetSystemTimes kernel time INCLUDES idle; process kernel time does not.
Process CPU is the sum across threads, so core equivalents can exceed one.
Multi-group hosts are refused because GetSystemTimes is group scoped there.
The trace covers a whole child process, not exact UI stages or GPU work.
"""
import ctypes
from ctypes import wintypes
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import subprocess
import time


class WindowsCpuSampler:
    def __init__(self, pid):
        if os.name != "nt":
            raise OSError("Windows CPU tracing requires Windows")
        self.api = ctypes.WinDLL("kernel32", use_last_error=True)
        signatures = {
            "GetSystemTimes": ([ctypes.POINTER(wintypes.FILETIME)] * 3, wintypes.BOOL),
            "GetProcessTimes": ([wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4, wintypes.BOOL),
            "OpenProcess": ([wintypes.DWORD, wintypes.BOOL, wintypes.DWORD], wintypes.HANDLE),
            "CloseHandle": ([wintypes.HANDLE], wintypes.BOOL),
            "GetCurrentProcess": ([], wintypes.HANDLE),
            "GetActiveProcessorGroupCount": ([], wintypes.WORD),
            "GetActiveProcessorCount": ([wintypes.WORD], wintypes.DWORD),
        }
        for name, (args, result) in signatures.items():
            function = getattr(self.api, name)
            function.argtypes, function.restype = args, result
        groups = self.api.GetActiveProcessorGroupCount()
        if groups != 1:
            raise OSError(f"CPU tracing supports one processor group; found {groups}")
        self.logical_cpus = self.api.GetActiveProcessorCount(0xFFFF)
        if not self.logical_cpus:
            raise ctypes.WinError(ctypes.get_last_error())
        self.process = self.api.OpenProcess(0x1000, False, pid)  # QUERY_LIMITED_INFORMATION
        if not self.process:
            raise ctypes.WinError(ctypes.get_last_error())
        self.observer = self.api.GetCurrentProcess()  # borrowed pseudo handle

    @staticmethod
    def _ticks(value):
        return (value.dwHighDateTime << 32) | value.dwLowDateTime

    def _process_times(self, handle):
        created, exited, kernel, user = (wintypes.FILETIME() for _ in range(4))
        if not self.api.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited),
                                        ctypes.byref(kernel), ctypes.byref(user)):
            raise ctypes.WinError(ctypes.get_last_error())
        return {"kernel_100ns": self._ticks(kernel), "user_100ns": self._ticks(user)}

    def snapshot(self):
        started = time.perf_counter_ns()
        idle, kernel, user = (wintypes.FILETIME() for _ in range(3))
        if not self.api.GetSystemTimes(ctypes.byref(idle), ctypes.byref(kernel), ctypes.byref(user)):
            raise ctypes.WinError(ctypes.get_last_error())
        system = {"idle_100ns": self._ticks(idle), "kernel_100ns": self._ticks(kernel),
                  "user_100ns": self._ticks(user)}
        process = self._process_times(self.process)
        observer = self._process_times(self.observer)
        ended = time.perf_counter_ns()
        return {"monotonic_ns": (started + ended) // 2, "read_span_ns": ended - started,
                "system": system, "process": process, "observer": observer}

    def close(self):
        if self.process:
            handle, self.process = self.process, None
            if not self.api.CloseHandle(handle):
                raise ctypes.WinError(ctypes.get_last_error())


def intervals(samples, logical_cpus):
    """Use measured intervals, retain raw counters, and reject invalid deltas."""
    if type(logical_cpus) is not int or logical_cpus < 1 or len(samples) < 2:
        raise ValueError("need a CPU count and at least two snapshots")
    for sample in samples:
        for name in ("monotonic_ns", "read_span_ns"):
            if type(sample[name]) is not int or sample[name] < 0:
                raise ValueError("invalid timestamp")
        for group in ("system", "process", "observer"):
            names = ("idle_100ns", "kernel_100ns", "user_100ns") if group == "system" else ("kernel_100ns", "user_100ns")
            for name in names:
                value = sample[group][name]
                if type(value) is not int or value < 0:
                    raise ValueError("invalid CPU counter")
    output = []
    for before, after in zip(samples, samples[1:]):
        wall_ms = (after["monotonic_ns"] - before["monotonic_ns"]) / 1_000_000
        if wall_ms <= 0:
            raise ValueError("non-increasing timestamp")
        deltas = {}
        for group in ("system", "process", "observer"):
            deltas[group] = {key: after[group][key] - value for key, value in before[group].items()}
            if any(value < 0 for value in deltas[group].values()):
                raise ValueError("CPU counter regressed")
        system = deltas["system"]
        total = system["kernel_100ns"] + system["user_100ns"]
        busy = total - system["idle_100ns"]
        if total < 0 or busy < 0:
            raise ValueError("invalid system CPU delta")
        row = {"start_monotonic_ns": before["monotonic_ns"],
               "end_monotonic_ns": after["monotonic_ns"], "wall_ms": wall_ms,
               "system_total_ms": total / 10_000, "system_busy_ms": busy / 10_000,
               "system_busy_pct": busy * 100 / total if total else None}
        for group in ("process", "observer"):
            cpu_ms = sum(deltas[group].values()) / 10_000
            row[group + "_cpu_ms"] = cpu_ms
            row[group + "_core_equivalents"] = cpu_ms / wall_ms
            row[group + "_capacity_pct"] = cpu_ms * 100 / wall_ms / logical_cpus
        output.append(row)
    return output


def summarize(rows, logical_cpus, requested_interval_s):
    if not rows or requested_interval_s <= 0:
        raise ValueError("missing trace intervals")
    wall = sum(row["wall_ms"] for row in rows)
    total = sum(row["system_total_ms"] for row in rows)
    observed_wall = sum(row["wall_ms"] for row in rows if row["system_busy_pct"] is not None)
    result = {"intervals": len(rows), "wall_ms": wall,
              "system_busy_pct": sum(row["system_busy_ms"] for row in rows) * 100 / total if total else None,
              "system_counter_wall_coverage": observed_wall / wall,
              "max_interval_ms": max(row["wall_ms"] for row in rows),
              "intervals_over_2x_requested": sum(row["wall_ms"] > requested_interval_s * 2000 for row in rows),
              "time_fraction_system_over_50pct": sum(row["wall_ms"] for row in rows if row["system_busy_pct"] is not None and row["system_busy_pct"] > 50) / observed_wall if observed_wall else None}
    for group in ("process", "observer"):
        cpu_ms = sum(row[group + "_cpu_ms"] for row in rows)
        result[group + "_cpu_ms"] = cpu_ms
        result[group + "_core_equivalents"] = cpu_ms / wall
        result[group + "_capacity_pct"] = cpu_ms * 100 / wall / logical_cpus
    return result


def run(command, *, cwd, env, log_path, trace_path, timeout_s=300, interval_s=0.1):
    """Run one owned child and record every observation, including timeout/error.

    interval_s=None disables CPU observation for calibration, retaining the same
    launch/log/wait path. No load-based waiting, sample filtering, or retries.
    Output paths must be new. Only the direct child is killed on timeout/error.
    """
    if os.name != "nt":
        raise OSError("Windows CPU tracing requires Windows")
    if not math.isfinite(timeout_s) or timeout_s <= 0:
        raise ValueError("invalid timeout")
    if interval_s is not None and (not math.isfinite(interval_s) or interval_s < 0.05):
        raise ValueError("sampling interval must be at least 50ms")
    log_path, trace_path = Path(log_path), Path(trace_path)
    if log_path.resolve() == trace_path.resolve():
        raise ValueError("log and trace paths must differ")
    # Exclusive opens refuse overwriting evidence before any child is launched.
    with trace_path.open("x", encoding="utf-8") as trace, log_path.open("x", encoding="utf-8") as log:
        started = time.perf_counter_ns()
        record = {"schema_version": 1, "started_utc": datetime.now(timezone.utc).isoformat(),
                  "command": [str(part) for part in command], "observer_pid": os.getpid(),
                  "observation_enabled": interval_s is not None, "requested_interval_s": interval_s,
                  "production_gate": "not_evaluated", "scope": "whole-process CPU, not UI-stage or GPU timing",
                  "raw_samples": [], "valid": False}
        child = sampler = None
        try:
            child = subprocess.Popen(command, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT,
                                     creationflags=subprocess.CREATE_NO_WINDOW)
            record["pid"] = child.pid
            if interval_s is not None:
                sampler = WindowsCpuSampler(child.pid)
                record["logical_cpus"] = sampler.logical_cpus
                record["raw_samples"].append(sampler.snapshot())
            deadline = time.monotonic() + timeout_s
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    child.kill()
                    child.wait(timeout=10)
                    record["timed_out"] = True
                    if sampler:
                        record["raw_samples"].append(sampler.snapshot())
                    break
                try:
                    child.wait(timeout=min(interval_s or remaining, remaining))
                except subprocess.TimeoutExpired:
                    pass
                if sampler:
                    record["raw_samples"].append(sampler.snapshot())
                if child.returncode is not None:
                    break
            if sampler:
                rows = intervals(record["raw_samples"], sampler.logical_cpus)
                record["intervals"] = rows
                record["summary"] = summarize(rows, sampler.logical_cpus, interval_s)
            record["exit_code"] = child.returncode
            record["valid"] = child.returncode == 0 and not record.get("timed_out", False)
        except Exception as error:
            record["error"] = f"{type(error).__name__}: {error}"
            if child is not None and child.poll() is None:
                child.kill()
                child.wait(timeout=10)
            record["exit_code"] = None if child is None else child.returncode
        finally:
            if sampler:
                try:
                    sampler.close()
                except OSError as error:
                    record["cleanup_error"] = str(error)
                    record["valid"] = False
            record["elapsed_ms"] = (time.perf_counter_ns() - started) / 1_000_000
            trace.write(json.dumps(record, indent=2))
        return record
