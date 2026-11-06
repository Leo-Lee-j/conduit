# P2-A：release 界面回归适配

日期：2026-11-06。承接 [G5 性能测量记录](p2-performance-probe-2026-11-06.md)中发现的 18 项 release 布局回归失败。本轮范围为定位修复、dev / release 常规回归和可重复执行的验证入口；不改变 P2 生产准入状态。

## 原因与修复

锁定版 GPUI 在 release 依赖构建中不记录 `debug_selector`，因此 `debug_bounds` 返回 `None`。此前正向查找会失败，部分只检查 `is_none()` 的隐藏断言则可能无效通过。

插件回归改用同一套 `ElementBounds` 查询：按钮及有 ID 的容器读取 `test_support::find` 的实际可见性和绘制边界；页签读取原生 `Tab` 角色及标签。查询拒绝歧义匹配，不再回退到调试标记。宽窄窗口、停靠宽度、控件不重叠、滚动偏移、隐藏、撤销、输入和焦点等断言继续执行。

宿主导航、工具栏、右侧容器、设置页和文件树补充 `.test_support()` 观察。已有元素沿用原 ID，缺少 ID 的布局容器补稳定 ID；观察不增加布局节点，在未启用 `test-support` 的正常构建中返回原元素。工作区概览和工具详情保留原有实例 ID，并以原生分组标签提供定位，继续隔离各工作区及工具的滚动状态。没有修改生成的 `.upstream`、依赖缓存、上游补丁或 release 的 debug assertions 配置。

## 可重复执行

```powershell
python tools/shell-prototype/verify-desktop.py --phase test --profile dev
python tools/shell-prototype/verify-desktop.py --phase test --profile release
python tools/shell-prototype/verify-desktop.py --phase check
python tools/shell-prototype/verify-desktop.py --phase release-check
```

`--profile` 现用于常规测试、检查、构建及性能采样；`release-check` 固定采用 release。可用 `--target-dir` 复用对应缓存。dev / release 数据目录、日志和带 profile 的 JSON 报告分开，原有 `desktop-<phase>-results.json` 继续保存该阶段最近一次结果。测试报告记录 passed / failed / ignored 数量，空筛选不能算通过；所有阶段均拒绝检查期间源码变化。

## 验证记录

验证使用 `target/g5-validation-20261106`，基于 `a121358`，仅带入此前 G5 探针及本轮改动。共享工作树中的其他设置、终端、侧栏及依赖改动不在此次结果范围内。

| 检查 | 结果 |
| --- | --- |
| release 插件回归 | 42 passed / 0 failed / 1 ignored |
| release 更新回归 | 3 passed / 0 failed |
| dev 插件回归 | 42 passed / 0 failed / 1 ignored |
| dev 更新回归 | 3 passed / 0 failed |
| 定向 rustfmt、all-targets + ui-tests check、Clippy | 全部通过，Clippy 使用 `-D warnings` |
| 普通 release 类型检查 | 通过，未启用 `ui-tests` / `test-support` |
| Python 性能报告校验 | 4 项通过 |

两个 profile 各 45 项常规测试通过；ignored 项为需要单独执行的性能探针。此前 18 项失败全部消除，没有开启 release debug assertions，也没有删除或放宽布局断言。最终 check / dev test / release test / 普通 release check 报告与当前隔离源码的 SHA-256 均为 `78f0b29340e37e81b70c1c1e51d8ca2b72a287f2449fe851a92be3472b8ac63d`，各轮前后摘要一致。

上游修订、补丁和 lockfile 沿用 G5 测量记录。报告与原始日志归档到 `tools/shell-prototype/artifacts/g5-release-ui-20261106/`；此前失败报告 `g5-release-regression-results.json` 保留历史含义。

## 剩余边界与下一步

常规回归在 GPUI 测试平台执行，没有原生 GPU 呈现。G4 的真实 Windows IME 候选框、组合输入事件和原生视觉验收仍待完成；上一轮原生连接不可用，本轮未重新执行原生验收。

下一步继续 G5：扩展代表性长会话、20 个未激活包、3 个活动插件及异常 / 正常插件并存的负载，增加重复激活 / 停止后的内存增长曲线与脚本、物化、HostModule 分项耗时，再完成 release 原生交互 / 滚动验收。此前性能样本仍只对应各自记录的源码快照，不代表本轮改动后的重新测量。完整准入通过后再冻结 P2 生产契约并进入 P3 安装管理。
