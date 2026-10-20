//! Find and replace in the transcript (Ctrl+F in the Captions tab). The search runs over the main
//! track's words joined by spaces, so a phrase matches across words and part of a word matches
//! too, as in a text editor; the interface finds with the same rules (`findMatches` in `app.js`).
//! Replacing changes the words of the media's transcript, keeping their times, and the captions
//! over them, in one undo step: nothing is cut from the video.

use std::collections::{BTreeMap, HashSet};

use filmcraft_edit::transcript::SeqWord;
use filmcraft_project::transcript::Transcript;
use filmcraft_project::{ClipId, ItemId};
use filmcraft_time::Tick;
use serde_json::json;

use crate::captions::{Layout, heard, main_words, same_words, wrap};
use crate::{Error, Result, Studio, engine};

/// One place the search text was found: words `first..=last`, and where in them it starts and
/// ends (characters into the first word, and into the last).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub first: usize,
    pub last: usize,
    pub from: usize,
    pub to: usize,
}

/// A character as the search compares it: lowercased unless the case must match (characters
/// whose lowercase is longer stay as they are, so positions line up).
fn fold(c: char, case: bool) -> char {
    if case {
        return c;
    }
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

/// Every place `find` (its spacing collapsed) appears in `words` joined by single spaces, left to
/// right without overlaps.
pub(crate) fn find_in(words: &[&str], find: &str, case: bool) -> Vec<Found> {
    let needle: Vec<char> = find.split_whitespace().collect::<Vec<_>>().join(" ").chars().map(|c| fold(c, case)).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    // the joined text, and which word (and where in it) each character is
    let mut text: Vec<char> = Vec::new();
    let mut at: Vec<Option<(usize, usize)>> = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            text.push(' ');
            at.push(None);
        }
        for (k, c) in w.chars().enumerate() {
            text.push(fold(c, case));
            at.push(Some((i, k)));
        }
    }
    let mut found = Vec::new();
    let mut i = 0;
    while i + needle.len() <= text.len() {
        if text.get(i..i + needle.len()).is_some_and(|s| s == needle.as_slice()) {
            // the needle has no space at either end, so both ends are in words
            if let (Some(Some((first, from))), Some(Some((last, to)))) = (at.get(i), at.get(i + needle.len() - 1)) {
                found.push(Found { first: *first, last: *last, from: *from, to: to + 1 });
            }
            i += needle.len();
        } else {
            i += 1;
        }
    }
    found
}

/// The words `f` covers once replaced: the first word's start, `with`, the last word's end.
fn replaced(words: &[&str], f: Found, with: &str) -> String {
    let head: String = words.get(f.first).map(|w| w.chars().take(f.from).collect()).unwrap_or_default();
    let tail: String = words.get(f.last).map(|w| w.chars().skip(f.to).collect()).unwrap_or_default();
    format!("{head}{with}{tail}")
}

/// What becomes of one word of a media transcript.
enum Change {
    Text(String),
    /// New text, and the media time it now ends at (it took in the words after it).
    Merge(String, Tick),
    Remove,
}

/// The result of a replace: how many places changed.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Replaced {
    pub count: usize,
}

impl Studio {
    /// Replaces `find` with `with` in the transcript and the captions over it: everywhere, or only
    /// at the `only`-th place it is found (0 = the first). Each word keeps its time; when the
    /// replacement has a different number of words, the found words become one word over their
    /// time. One undo step.
    pub fn replace_words(&mut self, find: &str, with: &str, case: bool, only: Option<usize>) -> Result<Replaced> {
        let q = self.session.active_sequence().ok_or(Error::NothingOpen)?;
        let old = main_words(q, &self.session.project);
        let texts: Vec<&str> = old.iter().map(|w| w.text.as_str()).collect();
        let mut found = find_in(&texts, find, case);
        if let Some(n) = only {
            found = found.get(n).copied().into_iter().collect();
        }
        let shown = find.split_whitespace().collect::<Vec<_>>().join(" ");
        if found.is_empty() && only.is_some() {
            return Err(Error::Invalid(format!("“{shown}” is no longer at that place in the transcript")));
        }
        let with = with.split_whitespace().collect::<Vec<_>>().join(" ");
        let layout = Layout::for_canvas(q.settings.width, q.settings.height);

        // what each found place does to the words of the media transcripts
        let mut changes: BTreeMap<(ItemId, usize), Change> = BTreeMap::new();
        let mut taken: HashSet<(ItemId, usize)> = HashSet::new();
        let mut count = 0;
        for f in &found {
            let span: Vec<&SeqWord> = old.get(f.first..=f.last).map(|s| s.iter().collect()).unwrap_or_default();
            // a media used twice on the main track is the same speech: change its words once
            if span.is_empty() || span.iter().any(|w| taken.contains(&(w.item, w.index))) {
                continue;
            }
            count += 1;
            taken.extend(span.iter().map(|w| (w.item, w.index)));
            let text = replaced(&texts, *f, &with);
            let tokens: Vec<&str> = text.split_whitespace().collect();
            if tokens.len() == span.len() {
                for (w, t) in span.iter().zip(&tokens) {
                    changes.insert((w.item, w.index), Change::Text((*t).to_string()));
                }
                continue;
            }
            let (Some(head), rest) = (span.first(), span.get(1..).unwrap_or_default()) else { continue };
            if tokens.is_empty() {
                changes.insert((head.item, head.index), Change::Remove);
            } else {
                // words heard one after the other in one media become one word over all their time
                let joined = rest.iter().enumerate().all(|(k, w)| w.item == head.item && w.index == head.index + k + 1);
                let end = rest.last().filter(|_| joined).and_then(|w| self.session.project.transcripts.get(&w.item)?.words.get(w.index).map(|x| x.end));
                let text = tokens.join(" ");
                changes.insert((head.item, head.index), end.map_or_else(|| Change::Text(text.clone()), |e| Change::Merge(text.clone(), e)));
            }
            for w in rest {
                changes.insert((w.item, w.index), Change::Remove);
            }
        }

        // the captions: ones that still say their words follow the words; ones typed over get the
        // same replacement in their own text
        let spans: Vec<(Tick, Tick)> = found.iter().filter_map(|f| Some((old.get(f.first)?.start, old.get(f.last)?.end))).collect();
        let captions: Vec<(ClipId, Tick, Tick, String, bool)> = q
            .caption_tracks
            .first()
            .filter(|t| !t.locked)
            .map(|t| t.captions.iter().map(|c| (c.id, c.start, c.end(), c.text.clone(), same_words(&c.text, &heard(&old, c.start, c.end())))).collect())
            .unwrap_or_default();

        let mut typed: Vec<(ClipId, String)> = Vec::new();
        for (id, start, end, text, auto) in &captions {
            if *auto {
                continue;
            }
            // replacing one place changes only a caption over it
            if only.is_some() && !spans.iter().any(|(a, b)| a < end && b > start) {
                continue;
            }
            let words: Vec<&str> = text.split_whitespace().collect();
            let mut hits = find_in(&words, find, case);
            if only.is_some() {
                hits.truncate(1);
            }
            if hits.is_empty() {
                continue;
            }
            // rebuilt from the right so earlier places stay where they were found
            let mut out: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
            for f in hits.iter().rev() {
                let refs: Vec<&str> = out.iter().map(String::as_str).collect();
                let t = replaced(&refs, *f, &with);
                out.splice(f.first..=f.last, [t]);
            }
            typed.push((*id, out.join(" ")));
        }
        if changes.is_empty() && typed.is_empty() {
            return Err(Error::Invalid(format!("“{shown}” isn't in the transcript")));
        }
        let count = count.max(typed.len());

        self.group("Replace", |s| {
            let mut by_item: BTreeMap<ItemId, Vec<(usize, &Change)>> = BTreeMap::new();
            for ((item, index), c) in &changes {
                by_item.entry(*item).or_default().push((*index, c));
            }
            for (item, list) in by_item {
                let Some(t) = s.session.project.transcripts.get(&item) else { continue };
                let mut t: Transcript = (**t).clone();
                for (index, c) in list {
                    let Some(w) = t.words.get_mut(index) else { continue };
                    match c {
                        Change::Text(x) => w.text.clone_from(x),
                        Change::Merge(x, end) => {
                            w.text.clone_from(x);
                            w.end = (*end).max(w.start);
                        }
                        // an empty word is dropped when the transcript is set
                        Change::Remove => w.text.clear(),
                    }
                }
                t.words.retain(|w| !w.text.trim().is_empty());
                let v = serde_json::to_value(&t).map_err(|e| Error::Engine(e.to_string()))?;
                s.session.execute("transcript.set", json!({"item": item.0, "transcript": v})).map_err(engine)?;
            }
            let q = s.session.active_sequence().ok_or(Error::NothingOpen)?;
            let new = main_words(q, &s.session.project);
            let mut gone = Vec::new();
            for (id, start, end, text, auto) in &captions {
                let next = if *auto {
                    heard(&new, *start, *end)
                } else if let Some((_, t)) = typed.iter().find(|(c, _)| c == id) {
                    t.clone()
                } else {
                    continue;
                };
                if next.trim().is_empty() {
                    gone.push(id.0);
                } else if !same_words(text, &next) {
                    s.session.execute("captions.setText", json!({"caption": id.0, "text": wrap(&next, layout)})).map_err(engine)?;
                }
            }
            if !gone.is_empty() {
                s.session.execute("captions.delete", json!({"captions": gone})).map_err(engine)?;
            }
            Ok(Replaced { count })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_runs_over_the_words_as_text() {
        let w = ["Hello", "there,", "New", "york.", "NEW", "York"];
        let f = find_in(&w, "new  YORK", false);
        assert_eq!(f, [Found { first: 2, last: 3, from: 0, to: 4 }, Found { first: 4, last: 5, from: 0, to: 4 }]);
        assert_eq!(replaced(&w, f[0], "New York"), "New York.");
        assert_eq!(find_in(&w, "new york", true), []);
        // part of a word, and across the space after a comma
        assert_eq!(find_in(&w, "ere, n", false), [Found { first: 1, last: 2, from: 2, to: 1 }]);
        assert_eq!(replaced(&w, find_in(&w, "ell", false)[0], "ELL"), "HELLo");
        // nothing to find, and found places never overlap
        assert!(find_in(&w, "  ", false).is_empty());
        assert_eq!(find_in(&["aaaa"], "aa", false).len(), 2);
        // a lowercase longer than its letter keeps the places lined up
        assert_eq!(find_in(&["İx", "y"], "x y", false), [Found { first: 0, last: 1, from: 1, to: 1 }]);
    }
}
