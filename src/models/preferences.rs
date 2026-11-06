//! Application-owned metadata; native CLI catalogs remain read-only.
use super::ModelOption;
use crate::domain::Provider;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPreferences {
    pub custom: Vec<ModelOption>,
    pub hidden: BTreeSet<String>,
}

impl ModelPreferences {
    pub fn merge(&self, native: &[ModelOption], include_hidden: bool) -> Vec<ModelOption> {
        let mut models = Vec::<ModelOption>::new();
        let mut positions = std::collections::HashMap::<&str, usize>::new();
        for model in native.iter().chain(&self.custom) {
            if let Some(&index) = positions.get(model.id.as_str()) {
                models[index] = model.clone();
            } else {
                positions.insert(&model.id, models.len());
                models.push(model.clone());
            }
        }
        if !include_hidden {
            models.retain(|model| !self.hidden.contains(&model.id));
        }
        models
    }

    pub fn save(
        &mut self,
        engine: Provider,
        original: Option<&str>,
        mut model: ModelOption,
    ) -> Result<()> {
        model.id = model.id.trim().to_owned();
        model.label = model.label.trim().to_owned();
        ensure!(
            !model.id.is_empty()
                && model.id.len() <= 256
                && !model.id.chars().any(char::is_whitespace)
                && !model.id.chars().any(char::is_control),
            "模型 ID 不能为空、包含空白或控制字符，且最多 256 字节"
        );
        ensure!(
            engine != Provider::OpenCode
                || model
                    .id
                    .split_once('/')
                    .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty()),
            "OpenCode 模型 ID 使用 provider/model 格式"
        );
        ensure!(
            model.label.len() <= 256 && !model.label.chars().any(char::is_control),
            "显示名称最多 256 字节，不能包含控制字符"
        );
        ensure!(
            model.note.len() <= 1200 && !model.note.chars().any(char::is_control),
            "备注最多 1200 字节，不能包含控制字符"
        );
        ensure!(
            model.context_window != Some(0),
            "上下文窗口必须为正整数或留空"
        );
        ensure!(
            !self
                .custom
                .iter()
                .any(|m| m.id == model.id && Some(m.id.as_str()) != original),
            "此模型 ID 已有本地定义，请编辑已有模型"
        );
        if model.label.is_empty() {
            model.label = model.id.clone();
        }
        if let Some(id) = original {
            self.custom.retain(|m| m.id != id);
        }
        self.custom.push(model);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(id: &str) -> ModelOption {
        ModelOption {
            id: id.into(),
            label: id.into(),
            note: String::new(),
            context_window: None,
        }
    }
    #[test]
    fn overlays_survive_refresh_and_roundtrip_without_mutating_native_models() {
        let native = vec![model("a"), model("b")];
        let mut prefs = ModelPreferences::default();
        let mut custom = model("a");
        custom.label = "Alias".into();
        prefs.save(Provider::Codex, None, custom).unwrap();
        prefs.hidden.insert("b".into());
        let restored: ModelPreferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert_eq!(restored.merge(&native, false).len(), 1);
        assert_eq!(restored.merge(&native, true)[0].label, "Alias");
        assert_eq!(native[0].label, "a");
        assert!(prefs.save(Provider::Codex, None, model("a")).is_err());
        prefs.custom.clear();
        assert_eq!(prefs.merge(&native, false)[0].label, "a");
    }
    #[test]
    fn invalid_model_ids_and_context_windows_are_rejected() {
        let mut prefs = ModelPreferences::default();
        for id in ["", "x\ny", "a b", "x\0y"] {
            assert!(prefs.save(Provider::Codex, None, model(id)).is_err());
        }
        assert!(
            prefs
                .save(Provider::OpenCode, None, model("unqualified"))
                .is_err()
        );
        prefs
            .save(Provider::OpenCode, None, model("vendor/model"))
            .unwrap();
        let mut invalid = model("valid");
        invalid.context_window = Some(0);
        assert!(prefs.save(Provider::Codex, None, invalid).is_err());
    }
}
