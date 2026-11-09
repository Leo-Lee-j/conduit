//! Group base-relative edits into aligned, independently reversible merge blocks.
use super::*;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Pending,
    Apply,
    Ignore,
}

#[derive(Clone, Debug)]
pub struct MergeBlock {
    pub base: String,
    pub ours: String,
    pub theirs: String,
    pub base_line: usize,
    pub ours_line: usize,
    pub theirs_line: usize,
    pub changed: bool,
    pub conflict: bool,
    pub decisions: [Decision; 2],
    /// Existing working-copy edits, retained until an explicit block reset.
    pub worktree: Option<String>,
}
impl MergeBlock {
    pub fn result(&self) -> String {
        if let Some(text) = &self.worktree {
            return text.clone();
        }
        match self.decisions {
            [Decision::Apply, Decision::Apply] if self.ours != self.theirs => {
                format!("{}{}", self.ours, self.theirs)
            }
            [Decision::Apply, _] => self.ours.clone(),
            [_, Decision::Apply] => self.theirs.clone(),
            _ => self.base.clone(),
        }
    }
    pub fn pending(&self) -> bool {
        self.worktree.is_some() || (self.changed && self.decisions.contains(&Decision::Pending))
    }
    pub fn reset(&mut self) {
        self.worktree = None;
        self.decisions = [
            if self.ours == self.base {
                Decision::Ignore
            } else {
                Decision::Pending
            },
            if self.theirs == self.base {
                Decision::Ignore
            } else {
                Decision::Pending
            },
        ];
    }
    pub fn apply_non_conflicting(&mut self) {
        if self.worktree.is_none() && self.changed && !self.conflict {
            if self.decisions[0] == Decision::Pending {
                self.decisions[0] = Decision::Apply;
            }
            if self.decisions[1] == Decision::Pending {
                self.decisions[1] = Decision::Apply;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct MergePlan {
    pub blocks: Vec<MergeBlock>,
}
#[derive(Debug)]
struct Edit {
    base: Range<usize>,
    replacement: String,
    side: usize,
}

fn line_range(value: &str) -> Result<Range<usize>> {
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    let start: usize = start.parse()?;
    let count: usize = count.parse()?;
    let offset = if count == 0 {
        start
    } else {
        start
            .checked_sub(1)
            .ok_or_else(|| anyhow!("Invalid diff range"))?
    };
    Ok(offset..offset + count)
}

fn edits(
    git: &Path,
    root: &Path,
    base: &str,
    text: &str,
    side: usize,
    cancel: &AtomicBool,
) -> Result<Vec<Edit>> {
    let scratch = Scratch::new_in(&std::env::temp_dir(), "merge-diff")?;
    let before = scratch.0.join("base");
    let after = scratch.0.join("side");
    fs::write(&before, base)?;
    fs::write(&after, text)?;
    let diff = run_git_bounded(
        git,
        root,
        &[
            "diff",
            "--no-index",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--text",
            "--unified=0",
            "--",
            &before.to_string_lossy(),
            &after.to_string_lossy(),
        ],
        cancel,
        4 * TEXT_LIMIT,
        COMMAND_TIMEOUT,
        true,
    )?;
    if diff.truncated {
        bail!(conduit::i18n::text("git_conflicts.too_large"));
    }
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let mut result = Vec::new();
    for header in diff.text.lines().filter(|line| line.starts_with("@@ -")) {
        let words: Vec<_> = header.split_whitespace().collect();
        let base = line_range(words[1].trim_start_matches('-'))?;
        let new = line_range(words[2].trim_start_matches('+'))?;
        let replacement = lines
            .get(new)
            .ok_or_else(|| anyhow!("Invalid diff range"))?
            .concat();
        result.push(Edit {
            base,
            replacement,
            side,
        });
    }
    Ok(result)
}

impl MergePlan {
    pub(super) fn preserving_worktree(
        base: String,
        ours: String,
        theirs: String,
        current: String,
    ) -> Result<Self> {
        if current.lines().count() > 30_000 {
            bail!(conduit::i18n::text("git_conflicts.too_large"));
        }
        let mut blocks = Vec::new();
        push(&mut blocks, base, Some((ours, theirs)));
        blocks[0].worktree = Some(current);
        Ok(Self { blocks })
    }

    pub(super) fn build(
        git: &Path,
        root: &Path,
        base: &str,
        ours: &str,
        theirs: &str,
        cancel: &AtomicBool,
    ) -> Result<Self> {
        let mut changes = edits(git, root, base, ours, 0, cancel)?;
        changes.extend(edits(git, root, base, theirs, 1, cancel)?);
        changes.sort_by_key(|edit| (edit.base.start, edit.base.end, edit.side));
        let lines: Vec<_> = base.split_inclusive('\n').collect();
        let mut blocks = Vec::new();
        let mut pos = 0;
        let mut index = 0;
        while index < changes.len() {
            let start = changes[index].base.start;
            let mut end = changes[index].base.end;
            if start > pos {
                push(&mut blocks, lines[pos..start].concat(), None);
            }
            let first = index;
            index += 1;
            // Insertions on a replacement boundary are conservatively reviewed
            // together. Adjacent nonempty replacements remain independent.
            while index < changes.len()
                && (changes[index].base.start < end
                    || (changes[index].base.start == end
                        && (changes[index].base.is_empty() || start == end)))
            {
                end = end.max(changes[index].base.end);
                index += 1;
            }
            let project = |side: usize| {
                let mut result = String::new();
                let mut cursor = start;
                for edit in changes[first..index]
                    .iter()
                    .filter(|edit| edit.side == side)
                {
                    result.push_str(&lines[cursor..edit.base.start].concat());
                    result.push_str(&edit.replacement);
                    cursor = edit.base.end;
                }
                result.push_str(&lines[cursor..end].concat());
                result
            };
            push(
                &mut blocks,
                lines[start..end].concat(),
                Some((project(0), project(1))),
            );
            pos = end;
        }
        if pos < lines.len() {
            push(&mut blocks, lines[pos..].concat(), None);
        }
        if blocks.is_empty() {
            push(&mut blocks, String::new(), None);
        }
        // Bound editor entities as well as bytes; whole-file selection remains
        // available for unusually fragmented or enormous files.
        if blocks.len() > 400
            || [base, ours, theirs]
                .iter()
                .any(|text| text.lines().count() > 30_000)
        {
            bail!(conduit::i18n::text("git_conflicts.too_large"));
        }
        let mut offsets = [1; 3];
        for block in &mut blocks {
            block.base_line = offsets[0];
            block.ours_line = offsets[1];
            block.theirs_line = offsets[2];
            for (i, text) in [&block.base, &block.ours, &block.theirs].iter().enumerate() {
                offsets[i] += text.lines().count();
            }
        }
        Ok(Self { blocks })
    }
}

fn push(blocks: &mut Vec<MergeBlock>, base: String, sides: Option<(String, String)>) {
    let changed = sides.is_some();
    let (ours, theirs) = sides.unwrap_or_else(|| (base.clone(), base.clone()));
    let conflict = ours != base && theirs != base && ours != theirs;
    let mut block = MergeBlock {
        base,
        ours,
        theirs,
        base_line: 1,
        ours_line: 1,
        theirs_line: 1,
        changed,
        conflict,
        decisions: [Decision::Ignore; 2],
        worktree: None,
    };
    block.reset();
    blocks.push(block);
}
