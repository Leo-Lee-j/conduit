//! Recover native snapshots without adding them to Conduit's per-turn ledger.
use crate::domain::{RequestUsage, Usage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

fn request(usage: Usage) -> RequestUsage {
    RequestUsage {
        input: usage.input_tokens_known.then_some(usage.input_tokens),
        cached: usage
            .cached_input_tokens_known
            .then_some(usage.cached_input_tokens),
        cache_write: usage.cache_write_input_tokens,
        output: usage.output_tokens_known.then_some(usage.output_tokens),
    }
}

pub(super) fn codex(usage: &mut Usage, info: &Value) {
    let total = &info["total_token_usage"];
    let last = &info["last_token_usage"];
    // Rate-limit-only events carry null info and must not erase a snapshot.
    if !total.is_object() && !last.is_object() {
        return;
    }
    let counts = |value: &Value| RequestUsage {
        input: value["input_tokens"].as_u64(),
        cached: value["cached_input_tokens"].as_u64(),
        cache_write: value["cache_write_input_tokens"].as_u64(),
        output: value["output_tokens"].as_u64(),
    };
    let latest = counts(last);
    let totals = counts(total);
    *usage = Usage {
        history: Some(latest),
        input_tokens_known: totals.input.is_some(),
        cached_input_tokens_known: totals.cached.is_some(),
        output_tokens_known: totals.output.is_some(),
        input_tokens: totals.input.unwrap_or(0),
        cached_input_tokens: totals.cached.unwrap_or(0),
        cache_write_input_tokens: totals.cache_write,
        output_tokens: totals.output.unwrap_or(0),
        total_tokens: total["total_tokens"].as_u64().unwrap_or_else(|| {
            totals
                .input
                .unwrap_or(0)
                .saturating_add(totals.output.unwrap_or(0))
        }),
        context_tokens: last["total_tokens"]
            .as_u64()
            .or_else(|| Some(latest.input?.saturating_add(latest.output?))),
        context_window: info["model_context_window"]
            .as_u64()
            .unwrap_or(usage.context_window),
        ..Default::default()
    };
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Claude {
    requests: HashMap<String, crate::protocol::ClaudeUsage>,
    latest: Option<String>,
    model: Option<String>,
}

impl Claude {
    pub fn accept(&mut self, envelope: &Value) {
        let message = &envelope["message"];
        let Some(id) = message["id"].as_str().filter(|id| !id.is_empty()) else {
            return;
        };
        if !message["usage"].is_object() {
            return;
        }
        if !self.requests.contains_key(id) {
            self.latest = Some(id.into());
            self.model = message["model"].as_str().map(str::to_owned);
        }
        // Multiple content blocks can repeat a message's usage, including a
        // later placeholder. Share the live parser's merge rules by message ID.
        self.requests
            .entry(id.into())
            .or_default()
            .merge(&message["usage"]);
    }

    pub fn finish(&self) -> Usage {
        let Some(latest) = self.latest.as_ref().and_then(|id| self.requests.get(id)) else {
            return Usage::default();
        };
        let latest = latest.snapshot();
        let mut total = Usage {
            input_tokens_known: true,
            cached_input_tokens_known: true,
            output_tokens_known: true,
            cache_write_input_tokens: Some(0),
            history: Some(request(latest)),
            // A known request size is independent of cumulative consumption.
            context_tokens: latest.input_tokens_known.then_some(latest.total_tokens),
            ..Default::default()
        };
        for usage in self.requests.values().map(|value| value.snapshot()) {
            add(&mut total, usage);
        }
        if let Some(window) = self
            .model
            .as_deref()
            .and_then(|model| crate::models::ModelCatalog::default().context_window(model))
        {
            total.context_window = window;
            total.context_window_estimated = true;
        }
        total
    }
}

fn add(total: &mut Usage, usage: Usage) {
    total.input_tokens_known &= usage.input_tokens_known;
    total.cached_input_tokens_known &= usage.cached_input_tokens_known;
    total.output_tokens_known &= usage.output_tokens_known;
    total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
    total.cached_input_tokens = total
        .cached_input_tokens
        .saturating_add(usage.cached_input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
    total.cache_write_input_tokens = total
        .cache_write_input_tokens
        .zip(usage.cache_write_input_tokens)
        .map(|(a, b)| a.saturating_add(b));
    total.total_tokens = total.input_tokens.saturating_add(total.output_tokens);
}

pub(super) fn opencode(total: &mut Usage, message: &Value) {
    if message["role"] != "assistant" || !message["tokens"].is_object() {
        return;
    }
    let tokens = &message["tokens"];
    let input = tokens["input"].as_u64();
    let cached = tokens["cache"]["read"].as_u64();
    let written = tokens["cache"]["write"].as_u64();
    let output = tokens["output"].as_u64();
    let latest = Usage {
        input_tokens_known: input.is_some() && cached.is_some() && written.is_some(),
        cached_input_tokens_known: cached.is_some(),
        output_tokens_known: output.is_some(),
        input_tokens: input
            .unwrap_or(0)
            .saturating_add(cached.unwrap_or(0))
            .saturating_add(written.unwrap_or(0)),
        cached_input_tokens: cached.unwrap_or(0),
        cache_write_input_tokens: written,
        output_tokens: output.unwrap_or(0),
        ..Default::default()
    };
    if total.history.is_none() {
        total.input_tokens_known = true;
        total.cached_input_tokens_known = true;
        total.output_tokens_known = true;
        total.cache_write_input_tokens = Some(0);
    }
    add(total, latest);
    total.history = Some(request(latest));
    total.context_tokens = latest.input_tokens_known.then_some(
        latest
            .input_tokens
            .saturating_add(latest.output_tokens)
            .saturating_add(tokens["reasoning"].as_u64().unwrap_or(0)),
    );
    // Native message records do not include the model's context-window limit.
    total.context_window = 0;
}
