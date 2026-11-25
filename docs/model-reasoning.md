# 模型思考能力清单

数据文件：[assets/model-reasoning.json](../assets/model-reasoning.json)。清单只描述能力，不会往用户的模型菜单中添加未配置的型号，也不代表账号拥有这些模型的访问权。

## 选择器如何匹配

1. Codex 原生通道优先使用该通道模型目录中的 `supported_reasoning_levels`。已配置 `model_catalog_json` 时读取该文件，否则读取存在的 `models_cache.json`。显式空列表也有权威性，不会用内置清单覆盖。
2. Claude/Codex 没有原生能力数据时，按当前模型 ID 和引擎匹配清单。配置里的 Claude 角色别名先解析到实际型号；没有映射的浮动别名不猜测版本。
3. OpenCode、Qoder 只使用当前会话发布的原生选项，并在模型切换后的配置回复中验证。清单里的 API 能力不直接转换成 ACP variant。
4. 当前 Gemini CLI 适配器没有设置思考档位的协议，因此即使清单记录了 Gemini API 的 `thinking_level`，界面也不会展示不可用的控制。
5. 未知模型、未确认的传输方式只提供“跟随默认”。旧会话中 Claude/Codex 不再受支持的值在发送前回到“跟随默认”，并同步保存。

Claude/Codex 的“跟随默认”不传显式强度；ACP 保留引擎当前值，OpenCode 发布原生 `default` 时使用它恢复默认。`none` 是关闭思考的真实档位，与“跟随默认”不同。ACP 中引擎发布的字面 `default` 也是独立原生选项，不与应用层的空选择混用。切换模型会清除上一个模型的强度选择。

匹配支持大小写归一化、`provider/model` 前缀、Claude `[1M]` 后缀和明确的日期快照后缀；不会让 `gpt-5` 匹配 `gpt-5-pro`、`gpt-5-custom` 或新的版本号。

## 维护清单

每组记录包含：

- `ids`：共享同一能力的明确模型 ID。不要添加覆盖未来版本的通配符。
- `control`：厂商原生参数或控制方式，区分档位、开关与 token 预算。
- 同一个型号可按不同控制方式分组，但同一引擎只能命中一组。例如 DeepSeek 的 Responses 接口可用 `none` 关闭思考，Claude CLI 的 `--effort` 只能传 `low/high/max`。
- `capabilities.levels`：真实生效的档位。不要把兼容接口接受但会降级的别名当成新档位。
- `capabilities.default`：已确认的模型默认值；不确定时填 `null`。此字段不会覆盖 CLI 配置。
- `engines`：当前适配器能够原样传递此控制的引擎。仅 Chat Completions 兼容不等于 Codex Responses 兼容。空数组表示只记录 API 能力，等待原生目录或适配器支持。
- `sources`、`verified_at`、`notes`：官方依据、核验日期及限制。CLI 特有的 `ultra` 等能力来自用户当前目录，不从 API 型号名推断。

修改 JSON 后重新构建即可随应用分发；运行中不会自动下载或覆盖此文件。新增厂商前同时核对模型能力和引擎传输，增加有实际区别的回归用例。当前覆盖 OpenAI、Anthropic、Gemini、DeepSeek、xAI、Kimi、GLM、Qwen 的 61 个明确型号，并不宣称穷尽全部模型。

验证命令：

```powershell
cargo test --locked --features ui-tests --bin conduit models:: -- --test-threads=1
cargo test --locked --features ui-tests --bin conduit model_picker -- --test-threads=1
```
