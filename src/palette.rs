//! Command palette (Ctrl+K): fuzzy-find a command by name and run it.
//!
//! The palette knows nothing about the commands themselves; the app passes
//! in a list of entries and gets back the index of the one chosen.

use eframe::egui::{self, Color32, FontId, Key, Modifiers, RichText, text::LayoutJob};

/// One row in the palette.
pub struct Entry {
    pub label: &'static str,
    /// Keyboard shortcut shown on the right, if any.
    pub shortcut: &'static str,
    /// Disabled entries are listed but can't be run (e.g. need a selection).
    pub enabled: bool,
}

#[derive(Default)]
pub struct Palette {
    open: bool,
    query: String,
    /// Highlighted row among the filtered results.
    cursor: usize,
    /// Focus the text field on the frame after opening.
    focus: bool,
}

impl Palette {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.cursor = 0;
        self.focus = true;
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// Show the palette if open. Returns the index (into `entries`) of the
    /// command to run, if one was chosen this frame.
    pub fn show(&mut self, ctx: &egui::Context, entries: &[Entry]) -> Option<usize> {
        if !self.open {
            return None;
        }
        let results = filter(&self.query, entries);
        self.cursor = self.cursor.min(results.len().saturating_sub(1));

        // Navigation keys are consumed before the text field sees them.
        let (up, down, enter) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        if !results.is_empty() {
            if up {
                self.cursor = (self.cursor + results.len() - 1) % results.len();
            }
            if down {
                self.cursor = (self.cursor + 1) % results.len();
            }
        }
        let mut chosen = enter.then(|| results.get(self.cursor).map(|r| r.index)).flatten();

        let area = egui::Modal::default_area(egui::Id::new("command_palette"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0));
        let modal = egui::Modal::new(egui::Id::new("command_palette"))
            .area(area)
            .backdrop_color(Color32::from_black_alpha(60))
            .show(ctx, |ui| {
                ui.set_width(460.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text("Type a command…")
                        .font(FontId::proportional(16.0))
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(&mut self.focus) {
                    edit.request_focus();
                }
                if edit.changed() {
                    self.cursor = 0;
                }
                ui.separator();
                if results.is_empty() {
                    ui.weak("No matching commands");
                }
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (row, r) in results.iter().enumerate() {
                        let entry = &entries[r.index];
                        let selected = row == self.cursor;
                        let resp = ui
                            .horizontal(|ui| {
                                let label = ui.add_enabled(
                                    entry.enabled,
                                    egui::Button::selectable(selected, highlighted(ui, entry.label, &r.matched)),
                                );
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(RichText::new(entry.shortcut).monospace().weak());
                                });
                                label
                            })
                            .inner;
                        if selected && (up || down) {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            chosen = Some(r.index);
                        }
                    }
                });
            });

        // Disabled commands can be highlighted but not run.
        let chosen = chosen.filter(|&i| entries[i].enabled);
        if chosen.is_some() || modal.should_close() {
            self.close();
        }
        chosen
    }
}

/// Label with the fuzzy-matched characters emphasized.
fn highlighted(ui: &egui::Ui, label: &str, matched: &[usize]) -> LayoutJob {
    let font = FontId::proportional(14.0);
    let normal = ui.visuals().text_color();
    let strong = ui.visuals().strong_text_color();
    let mut job = LayoutJob::default();
    for (i, c) in label.chars().enumerate() {
        let hit = matched.contains(&i);
        let format = egui::TextFormat {
            font_id: font.clone(),
            color: if hit { strong } else { normal },
            underline: if hit { egui::Stroke::new(1.0, strong) } else { egui::Stroke::NONE },
            ..Default::default()
        };
        job.append(&c.to_string(), 0.0, format);
    }
    job
}

struct Match {
    index: usize,
    matched: Vec<usize>,
}

/// Entries matching `query`, best first; all entries in order for an empty query.
fn filter(query: &str, entries: &[Entry]) -> Vec<Match> {
    let mut scored: Vec<(i32, Match)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, e)| fuzzy(query, e.label).map(|(score, matched)| (score, Match { index, matched })))
        .collect();
    // Stable sort keeps the list order for equal scores.
    scored.sort_by_key(|(score, _)| -score);
    scored.into_iter().map(|(_, m)| m).collect()
}

/// Case-insensitive subsequence match of `query` in `text`. Returns a score
/// (higher is better) and the matched character indices. Consecutive
/// characters and matches at word starts score higher; gaps cost a little.
pub fn fuzzy(query: &str, text: &str) -> Option<(i32, Vec<usize>)> {
    let text: Vec<char> = text.chars().collect();
    let mut matched = Vec::new();
    let mut score = 0;
    let mut ti = 0;
    for q in query.chars().filter(|c| !c.is_whitespace()) {
        let q = q.to_ascii_lowercase();
        let found = (ti..text.len()).find(|&i| text[i].to_ascii_lowercase() == q)?;
        let word_start = found == 0 || !text[found - 1].is_alphanumeric();
        let consecutive = matched.last().is_some_and(|&last| last + 1 == found);
        score += 1;
        if word_start {
            score += 8;
        }
        if consecutive {
            score += 5;
        }
        score -= (found - ti).min(5) as i32;
        matched.push(found);
        ti = found + 1;
    }
    Some((score, matched))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(query: &str, names: &[&'static str]) -> Vec<&'static str> {
        let entries: Vec<Entry> =
            names.iter().map(|&label| Entry { label, shortcut: "", enabled: true }).collect();
        filter(query, &entries).into_iter().map(|m| names[m.index]).collect()
    }

    #[test]
    fn subsequence_and_case() {
        assert!(fuzzy("fsw", "Free sewing").is_some());
        assert!(fuzzy("FREE", "free sewing").is_some());
        assert!(fuzzy("xyz", "Free sewing").is_none());
        assert_eq!(fuzzy("", "anything").map(|m| m.1), Some(vec![]));
        assert_eq!(fuzzy("fs", "Free sewing").unwrap().1, vec![0, 5]);
    }

    #[test]
    fn word_starts_and_runs_rank_first() {
        let names = ["Redo", "Rotate 90° clockwise", "Draw piece", "Straighten selected segments"];
        assert_eq!(labels("rot", &names)[0], "Rotate 90° clockwise");
        assert_eq!(labels("dp", &names)[0], "Draw piece");
        assert_eq!(labels("st", &names)[0], "Straighten selected segments");
    }

    #[test]
    fn empty_query_keeps_order() {
        let names = ["b", "a", "c"];
        assert_eq!(labels("", &names), names);
    }
}
