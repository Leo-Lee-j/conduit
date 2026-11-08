#!/usr/bin/env python3
"""Validate embedded catalogs without compiling the desktop application."""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON key: {key}")
        result[key] = value
    return result

def read(path):
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_object)

def parameters(text):
    return set(re.findall(r"%\{([^}]+)\}", text))

def main():
    languages = read(ROOT / "assets/languages.json")
    ids = [language["id"] for language in languages]
    assert len(ids) == len(set(ids)), "Duplicate language ID"
    assert "en" in ids, "English fallback is required"
    baseline = read(ROOT / "locales/en.json")
    for language in languages:
        assert re.fullmatch(r"[A-Za-z0-9-]+", language["id"]), "Invalid locale ID"
        assert language["name"].strip(), "Language name is empty"
        translated = read(ROOT / "locales" / (language["id"] + ".json"))
        assert baseline.keys() == translated.keys(), f"Key mismatch: {language['id']}"
        for key, text in baseline.items():
            if key == "_version":
                assert translated[key] == text
                continue
            actual = translated[key]
            assert isinstance(actual, str) and actual, f"Empty translation: {language['id']}:{key}"
            assert parameters(actual) == parameters(text), f"Parameter mismatch: {language['id']}:{key}"
    for key in read(ROOT / "assets/i18n-sources.json").values():
        assert key in baseline, f"Unknown source alias target: {key}"
    for source in (ROOT / "src").rglob("*.rs"):
        if source.name == "i18n.rs":
            continue
        for key in re.findall(r'i18n::(?:text_static|text|format)\(\s*"([^"\n]+)"', source.read_text(encoding="utf-8")):
            assert key in baseline, f"Unknown message key {key} in {source}"
    print(f"Validated {len(ids)} languages and {len(baseline) - 1} messages per language.")

if __name__ == "__main__":
    main()
