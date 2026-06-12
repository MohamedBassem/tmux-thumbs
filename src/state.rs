use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fmt;

lazy_static! {
  static ref EXCLUDE_REGEXES: Vec<(&'static str, Regex)> = EXCLUDE_PATTERNS
    .iter()
    .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
    .collect();
  static ref PATTERN_REGEXES: Vec<(&'static str, Regex)> = PATTERNS
    .iter()
    .map(|tuple| (tuple.0, Regex::new(tuple.1).unwrap()))
    .collect();
  static ref LISTING_COMMAND_RE: Regex = Regex::new(r"(^|[^\w.-])(ls|eza)(\s|$)").unwrap();
  static ref GIT_STATUS_COMMAND_RE: Regex =
    Regex::new(r"(^|[^[:alnum:]_.-])git[[:space:]]+status([[:space:]]|$)").unwrap();
  static ref NEXT_PROMPT_RE: Regex =
    Regex::new(r"(^|[^\w.-])(git|cd|cat|echo|vim|nvim|less|tail|grep|rg|cargo|npm|pnpm|yarn)(\s|$)").unwrap();
  static ref LONG_LISTING_PERMS_RE: Regex = Regex::new(r"^[bcdlps.-][rwxStTs-]{9}").unwrap();
  static ref NON_WHITESPACE_RE: Regex = Regex::new(r"\S+").unwrap();
  static ref LISTING_SEPARATOR_RE: Regex = Regex::new(r"\s{2,}|\t+").unwrap();
  static ref ANSI_RE: Regex = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
}

const EXCLUDE_PATTERNS: [(&'static str, &'static str); 1] = [("bash", r"[[:cntrl:]]\[([0-9]{1,2};)?([0-9]{1,2})?m")];

const PATTERNS: [(&'static str, &'static str); 15] = [
  ("markdown_url", r"\[[^]]*\]\(([^)]+)\)"),
  ("url", r"(?P<match>(https?://|git@|git://|ssh://|ftp://|file:///)[^ ]+)"),
  (
    "diff_summary",
    r"diff --git a/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++) b/([.\w\-@~\[\]]+?/[.\w\-@\[\]]++)",
  ),
  ("diff_a", r"--- a/([^ ]+)"),
  ("diff_b", r"\+\+\+ b/([^ ]+)"),
  ("docker", r"sha256:([0-9a-f]{64})"),
  ("path", r"(?P<match>([.\w\-@$~\[\]]+)?(/[.\w\-@$\[\]]+)+)"),
  ("color", r"#[0-9a-fA-F]{6}"),
  ("uid", r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}"),
  ("ipfs", r"Qm[0-9a-zA-Z]{44}"),
  ("sha", r"[0-9a-f]{7,40}"),
  ("ip", r"\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}"),
  ("ipv6", r"[A-f0-9:]+:+[A-f0-9:]+[%\w\d]+"),
  ("address", r"0x[0-9a-fA-F]+"),
  ("number", r"[0-9]{4,}"),
];

#[derive(Clone)]
pub struct Match<'a> {
  pub x: i32,
  pub y: i32,
  pub pattern: &'a str,
  pub text: &'a str,
  pub hint: Option<String>,
}

impl<'a> fmt::Debug for Match<'a> {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    write!(
      f,
      "Match {{ x: {}, y: {}, pattern: {}, text: {}, hint: <{}> }}",
      self.x,
      self.y,
      self.pattern,
      self.text,
      self.hint.clone().unwrap_or("<undefined>".to_string())
    )
  }
}

impl<'a> PartialEq for Match<'a> {
  fn eq(&self, other: &Match) -> bool {
    self.x == other.x && self.y == other.y
  }
}

pub struct State<'a> {
  pub lines: &'a Vec<&'a str>,
  alphabet: &'a str,
  regexp: &'a Vec<&'a str>,
}

impl<'a> State<'a> {
  pub fn new(lines: &'a Vec<&'a str>, alphabet: &'a str, regexp: &'a Vec<&'a str>) -> State<'a> {
    State {
      lines,
      alphabet,
      regexp,
    }
  }

  pub fn matches(&self, reverse: bool, _unique: bool) -> Vec<Match<'a>> {
    let (listing_lines, listing_context_lines) = self.listing_context_indexes();
    let (git_status_lines, git_status_context_lines) = self.git_status_context_indexes();
    let mut matches = self.command_matches(&listing_lines);
    matches.extend(self.git_status_matches(&git_status_lines));

    let custom_patterns = self
      .regexp
      .iter()
      .map(|regexp| ("custom", Regex::new(regexp).expect("Invalid custom regexp")))
      .collect::<Vec<_>>();

    // This order determines the priority of pattern matching
    let all_patterns = EXCLUDE_REGEXES
      .iter()
      .map(|(name, regex)| (*name, regex))
      .chain(custom_patterns.iter().map(|(name, regex)| (*name, regex)))
      .chain(PATTERN_REGEXES.iter().map(|(name, regex)| (*name, regex)))
      .collect::<Vec<(&str, &Regex)>>();

    for (index, line) in self.lines.iter().enumerate() {
      if listing_lines.contains(&index)
        || listing_context_lines.contains(&index)
        || git_status_lines.contains(&index)
        || git_status_context_lines.contains(&index)
      {
        continue;
      }

      let mut chunk: &str = line;
      let mut offset: i32 = 0;

      loop {
        // For this line we search the first match of each pattern, then keep
        // the one with the lowest start index.
        let first_match_option = all_patterns
          .iter()
          .filter_map(|(name, regex)| regex.find(chunk).map(|matching| (*name, *regex, matching)))
          .min_by_key(|(_, _, matching)| matching.start());

        if let Some(first_match) = first_match_option {
          let (name, pattern, matching) = &first_match;
          let text = matching.as_str();

          if let Some(captures) = pattern.captures(text) {
            let captures: Vec<(&str, usize)> = if let Some(capture) = captures.name("match") {
              [(capture.as_str(), capture.start())].to_vec()
            } else if captures.len() > 1 {
              captures
                .iter()
                .skip(1)
                .filter_map(|capture| capture)
                .map(|capture| (capture.as_str(), capture.start()))
                .collect::<Vec<(&str, usize)>>()
            } else {
              [(matching.as_str(), 0)].to_vec()
            };

            // Never hint or broke bash color sequences, but process it
            if *name != "bash" {
              for (subtext, substart) in captures.iter() {
                let x = offset + matching.start() as i32 + *substart as i32;

                if !Self::overlaps_existing_match(&matches, index as i32, x, subtext.len() as i32) {
                  matches.push(Match {
                    x,
                    y: index as i32,
                    pattern: name,
                    text: subtext,
                    hint: None,
                  });
                }
              }
            }

            chunk = chunk.get(matching.end()..).expect("Unknown chunk");
            offset += matching.end() as i32;
          } else {
            panic!("No matching?");
          }
        } else {
          break;
        }
      }
    }

    self.assign_hints(&mut matches, reverse);

    matches
  }

  fn assign_hints(&self, matches: &mut Vec<Match<'a>>, reverse: bool) {
    let alphabet = super::alphabets::get_alphabet(self.alphabet);
    let mut hints = alphabet.hints(matches.len());
    let available_hints = hints.iter().cloned().collect::<HashSet<String>>();
    let has_multi_character_hints = hints.iter().any(|hint| hint.chars().count() > 1);
    let reserved_prefixes = Self::reserved_hint_prefixes(&hints);

    // This looks wrong but we do a pop after
    if !reverse {
      hints.reverse();
    } else {
      matches.reverse();
      hints.reverse();
    }

    let mut assigned_by_text: HashMap<&str, String> = HashMap::new();
    let mut used_hints: HashSet<String> = HashSet::new();

    for mat in matches.iter_mut() {
      if let Some(previous_hint) = assigned_by_text.get(mat.text) {
        mat.hint = Some(previous_hint.clone());
        continue;
      }

      if let Some(preferred_hint) = self.preferred_hint_for_match(mat, alphabet.letters()) {
        if (!has_multi_character_hints || available_hints.contains(&preferred_hint))
          && !used_hints.contains(&preferred_hint)
          && !reserved_prefixes.contains(&preferred_hint)
        {
          hints.retain(|hint| hint != &preferred_hint);
          used_hints.insert(preferred_hint.clone());
          assigned_by_text.insert(mat.text, preferred_hint.clone());
          mat.hint = Some(preferred_hint);
          continue;
        }
      }

      while let Some(hint) = hints.pop() {
        if used_hints.insert(hint.clone()) {
          assigned_by_text.insert(mat.text, hint.clone());
          mat.hint = Some(hint);
          break;
        }
      }
    }

    if reverse {
      matches.reverse();
    }
  }

  fn command_matches(&self, listing_lines: &HashSet<usize>) -> Vec<Match<'a>> {
    let mut matches = Vec::new();
    let mut indexes = listing_lines.iter().collect::<Vec<_>>();

    indexes.sort();

    for index in indexes {
      if let Some(line) = self.lines.get(*index) {
        matches.extend(Self::listing_line_matches(line, *index as i32));
      }
    }

    matches
  }

  fn reserved_hint_prefixes(hints: &[String]) -> HashSet<String> {
    let mut prefixes = HashSet::new();

    for hint in hints {
      let mut prefix = String::new();
      for ch in hint.chars().take(hint.chars().count().saturating_sub(1)) {
        prefix.push(ch);
        prefixes.insert(prefix.clone());
      }
    }

    prefixes
  }

  fn preferred_hint_for_match(&self, mat: &Match, alphabet: &str) -> Option<String> {
    let line = self.lines.get(mat.y as usize)?;
    let ch = line.get(mat.x as usize..)?.chars().next()?;
    let normalized = ch.to_lowercase().next().unwrap_or(ch);

    alphabet
      .chars()
      .find(|letter| letter.to_lowercase().next().unwrap_or(*letter) == normalized)
      .map(|letter| letter.to_string())
  }

  fn listing_context_indexes(&self) -> (HashSet<usize>, HashSet<usize>) {
    let mut indexes = HashSet::new();
    let mut context = HashSet::new();
    let mut in_listing = false;

    for (index, line) in self.lines.iter().enumerate() {
      if Self::is_listing_command(line) {
        in_listing = true;
        context.insert(index);
        continue;
      }

      if !in_listing {
        continue;
      }

      if line.trim().is_empty() {
        in_listing = false;
        continue;
      }

      if Self::looks_like_next_prompt(line) {
        in_listing = false;
        context.insert(index);
        continue;
      }

      indexes.insert(index);
    }

    (indexes, context)
  }

  fn git_status_context_indexes(&self) -> (HashSet<usize>, HashSet<usize>) {
    let mut indexes = HashSet::new();
    let mut context = HashSet::new();
    let mut in_status = false;

    for (index, line) in self.lines.iter().enumerate() {
      if Self::is_git_status_command(line) {
        in_status = true;
        context.insert(index);
        continue;
      }

      if !in_status {
        continue;
      }

      if line.trim().is_empty() {
        context.insert(index);
        continue;
      }

      if Self::git_status_line_match(line, index as i32).is_some() {
        indexes.insert(index);
      } else if !Self::starts_with_whitespace(line) && Self::looks_like_next_prompt(line) {
        in_status = false;
        context.insert(index);
      } else {
        context.insert(index);
      }
    }

    (indexes, context)
  }

  fn is_listing_command(line: &str) -> bool {
    let clean = Self::strip_ansi(line);
    let trimmed = clean.trim();

    if trimmed.starts_with("total ") {
      return false;
    }

    LISTING_COMMAND_RE.is_match(trimmed)
  }

  fn is_git_status_command(line: &str) -> bool {
    let clean = Self::strip_ansi(line);
    let trimmed = clean.trim();

    if trimmed.contains("git status") {
      return true;
    }

    GIT_STATUS_COMMAND_RE.is_match(trimmed)
  }

  fn looks_like_next_prompt(line: &str) -> bool {
    let clean = Self::strip_ansi(line);
    let trimmed = clean.trim();

    Self::is_listing_command(trimmed) || trimmed.contains('❯') || NEXT_PROMPT_RE.is_match(trimmed)
  }

  fn listing_line_matches(line: &'a str, y: i32) -> Vec<Match<'a>> {
    if line.trim_start().starts_with("total ") {
      return vec![];
    }

    if let Some(mat) = Self::long_listing_match(line, y) {
      return vec![mat];
    }

    Self::column_listing_matches(line, y)
  }

  fn git_status_matches(&self, git_status_lines: &HashSet<usize>) -> Vec<Match<'a>> {
    let mut matches = Vec::new();
    let mut indexes = git_status_lines.iter().collect::<Vec<_>>();

    indexes.sort();

    for index in indexes {
      if let Some(line) = self.lines.get(*index) {
        if let Some(mat) = Self::git_status_line_match(line, *index as i32) {
          matches.push(mat);
        }
      }
    }

    matches
  }

  fn git_status_line_match(line: &'a str, y: i32) -> Option<Match<'a>> {
    let trimmed = line.trim();

    if trimmed.is_empty() || trimmed.starts_with('(') {
      return None;
    }

    let path = if let Some(colon) = trimmed.find(':') {
      let status = trimmed[..colon].trim();
      if !matches!(
        status,
        "modified" | "deleted" | "new file" | "renamed" | "copied" | "both modified" | "both added" | "both deleted"
      ) {
        return None;
      }

      trimmed[colon + 1..].trim()
    } else if Self::starts_with_whitespace(line) {
      trimmed
    } else {
      return None;
    };

    if path.is_empty() || path.starts_with('(') {
      return None;
    }

    let path = if let Some(arrow) = path.rfind(" -> ") {
      &path[arrow + 4..]
    } else {
      path
    };

    let path = path.trim_matches('"');
    let start = line.find(path)?;

    Some(Match {
      x: start as i32,
      y,
      pattern: "git_status",
      text: path,
      hint: None,
    })
  }

  fn long_listing_match(line: &'a str, y: i32) -> Option<Match<'a>> {
    let trimmed_start = line.trim_start();

    if !LONG_LISTING_PERMS_RE.is_match(trimmed_start) {
      return None;
    }

    let tokens = NON_WHITESPACE_RE.find_iter(line).collect::<Vec<_>>();

    if tokens.len() < 6 {
      return None;
    }

    let name_token_index = if tokens.len() >= 9 { 8 } else { tokens.len() - 1 };
    let name_start = tokens[name_token_index].start();
    let name_end = line[name_start..]
      .find(" -> ")
      .map(|offset| name_start + offset)
      .unwrap_or_else(|| line.trim_end().len());

    if name_end <= name_start {
      return None;
    }

    Some(Match {
      x: name_start as i32,
      y,
      pattern: "listing",
      text: &line[name_start..name_end],
      hint: None,
    })
  }

  fn column_listing_matches(line: &'a str, y: i32) -> Vec<Match<'a>> {
    let mut matches = Vec::new();
    let mut start = 0;

    for separator_match in LISTING_SEPARATOR_RE.find_iter(line) {
      Self::push_listing_column_match(&mut matches, line, y, start, separator_match.start());
      start = separator_match.end();
    }

    Self::push_listing_column_match(&mut matches, line, y, start, line.len());
    matches
  }

  fn push_listing_column_match(matches: &mut Vec<Match<'a>>, line: &'a str, y: i32, start: usize, end: usize) {
    let text = &line[start..end];
    let trimmed = text.trim_matches(|ch: char| ch.is_whitespace());

    if trimmed.is_empty() {
      return;
    }

    let trimmed_start = text.find(trimmed).unwrap_or(0);
    let (icon_offset, entry) = Self::trim_listing_icon(Self::trim_listing_indicator(trimmed));
    let entry_start = start + trimmed_start + icon_offset;

    if entry.is_empty() {
      return;
    }

    matches.push(Match {
      x: entry_start as i32,
      y,
      pattern: "listing",
      text: entry,
      hint: None,
    });
  }

  fn trim_listing_indicator(text: &'a str) -> &'a str {
    text.trim_end_matches(|ch| ch == '*' || ch == '/' || ch == '@' || ch == '=' || ch == '|')
  }

  fn trim_listing_icon(text: &'a str) -> (usize, &'a str) {
    let mut chars = text.char_indices();

    if let Some((_, first)) = chars.next() {
      if !first.is_ascii_alphanumeric() && first != '.' {
        if let Some((space_index, ch)) = chars.next() {
          if ch.is_whitespace() {
            let entry_start = text[space_index..]
              .find(|candidate: char| !candidate.is_whitespace())
              .map(|offset| space_index + offset)
              .unwrap_or(text.len());

            return (entry_start, &text[entry_start..]);
          }
        }
      }
    }

    (0, text)
  }

  fn strip_ansi(line: &str) -> std::borrow::Cow<str> {
    // Most terminal lines carry no escape sequences; skip the regex pass and
    // the allocation entirely in that (very common) case.
    if line.as_bytes().contains(&0x1b) {
      ANSI_RE.replace_all(line, "")
    } else {
      std::borrow::Cow::Borrowed(line)
    }
  }

  fn starts_with_whitespace(line: &str) -> bool {
    line.chars().next().map(|ch| ch.is_whitespace()).unwrap_or(false)
  }

  fn overlaps_existing_match(matches: &[Match], y: i32, x: i32, len: i32) -> bool {
    matches.iter().any(|mat| {
      let mat_start = mat.x;
      let mat_end = mat.x + mat.text.len() as i32;
      let start = x;
      let end = x + len;

      mat.y == y && start < mat_end && mat_start < end
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn split(output: &str) -> Vec<&str> {
    output.split("\n").collect::<Vec<&str>>()
  }

  fn bench_corpus() -> String {
    // A representative terminal capture: prompts, ls output, git status,
    // diffs, urls, paths, shas, ips and plain prose.
    let block = "\
user@host ~/projects/tmux-thumbs ❯ ls
Cargo.toml  Cargo.lock  README.md  LICENSE  src  samples  scripts  target
user@host ~/projects/tmux-thumbs ❯ ls -la src
total 104
-rw-r--r-- 1 me staff  2985 Jun 12 07:25 alphabets.rs
-rw-r--r-- 1 me staff  1708 Jun 12 07:25 colors.rs
-rw-r--r-- 1 me staff  7320 Jun 12 07:25 main.rs
-rw-r--r-- 1 me staff 32463 Jun 12 07:25 state.rs
lrwxr-xr-x 1 me staff     3 Jun 12 07:25 link -> ../target
user@host ~/projects/tmux-thumbs ❯ git status
On branch master
Changes not staged for commit:
  (use \"git add <file>...\" to update what will be committed)
	modified:   src/state.rs
	modified:   src/view.rs
	renamed:    old.txt -> new.txt
Untracked files:
	src/bench.rs
user@host ~/projects/tmux-thumbs ❯ cat notes.txt
Visit https://github.com/fcsonline/tmux-thumbs for docs and ssh://git@host/repo.
The server 192.168.1.42 responded, see /var/log/nginx/access.log for details.
Commit fd70b5695a8c4e1f9d3b2a1c0e7f6d5c4b3a2918 fixed the 0xdeadbeef issue.
sha256:30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4 layer cached
The color #ff8800 and uuid 123e4567-e89b-12d3-a456-426655440000 appear in [link](https://example.com/path).
user@host ~/projects/tmux-thumbs ❯ git diff
diff --git a/src/state.rs b/src/state.rs
--- a/src/state.rs
+++ b/src/state.rs
Just some plain prose line without much to match here at all really.
";
    block.repeat(40)
  }

  #[test]
  #[ignore]
  fn bench_matches() {
    use std::time::Instant;

    let corpus = bench_corpus();
    let lines = corpus.split('\n').collect::<Vec<&str>>();
    let custom = ["CUSTOM-[0-9]{4,}", "ISSUE-[0-9]{3}"].to_vec();

    // Warm up.
    let warm = State::new(&lines, "qwerty", &custom).matches(false, false);
    let match_count = warm.len();

    let iterations = 50;
    let start = Instant::now();
    for _ in 0..iterations {
      let state = State::new(&lines, "qwerty", &custom);
      let results = state.matches(false, false);
      assert_eq!(results.len(), match_count);
    }
    let elapsed = start.elapsed();

    eprintln!(
      "bench_matches: {} lines, {} matches, {} iters, {:.3} ms/iter",
      lines.len(),
      match_count,
      iterations,
      elapsed.as_secs_f64() * 1000.0 / iterations as f64
    );
  }

  #[test]
  fn match_reverse() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "a");
  }

  #[test]
  fn match_unique() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, true);

    assert_eq!(results.len(), 3);
    assert_eq!(results.first().unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.last().unwrap().hint.clone().unwrap(), "a");
  }

  #[test]
  fn exact_matches_share_hints_without_unique_mode() {
    let lines = split("lorem 127.0.0.1 lorem 255.255.255.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.get(0).unwrap().hint.clone().unwrap(), results.get(2).unwrap().hint.clone().unwrap());
  }

  #[test]
  fn preferred_hints_use_match_start_character_with_deterministic_ties() {
    let lines = split("Cat Car Dog");
    let custom = [r"[A-Z][a-z]+"].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "Cat");
    assert_eq!(results.get(0).unwrap().hint.clone().unwrap(), "c");
    assert_eq!(results.get(1).unwrap().text, "Car");
    assert_eq!(results.get(1).unwrap().hint.clone().unwrap(), "a");
    assert_eq!(results.get(2).unwrap().text, "Dog");
    assert_eq!(results.get(2).unwrap().hint.clone().unwrap(), "d");
  }

  #[test]
  fn preferred_hints_do_not_shadow_multi_character_hints() {
    let lines = split("Dog Ape Bat Cow Emu Fox");
    let custom = [r"[A-Z][a-z]+"].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 6);
    assert_ne!(results.get(0).unwrap().hint.clone().unwrap(), "d");
    assert!(results.iter().any(|mat| mat.hint.clone().unwrap().starts_with('d')));
  }

  #[test]
  fn preferred_hints_do_not_allocate_keys_outside_generated_hints() {
    let lines = split("Ace Bat Cat Dog Egg Fox");
    let custom = [r"[A-Z][a-z]+"].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);
    let hints = results.iter().map(|mat| mat.hint.clone().unwrap()).collect::<Vec<_>>();

    assert_eq!(hints, ["a", "b", "c", "da", "db", "dc"]);
  }

  #[test]
  fn git_ls_style_paths_still_get_multi_character_hints() {
    let lines = split(
      "src/a.rs src/b.rs src/c.rs src/d.rs src/e.rs src/f.rs src/g.rs src/h.rs src/i.rs src/j.rs src/k.rs src/l.rs src/m.rs src/n.rs src/o.rs src/p.rs src/q.rs src/r.rs src/s.rs src/t.rs src/u.rs src/v.rs src/w.rs src/x.rs src/y.rs src/z.rs src/aa.rs src/ab.rs",
    );
    let custom = [].to_vec();
    let results = State::new(&lines, "qwerty", &custom).matches(false, false);
    let hints = results.iter().filter_map(|mat| mat.hint.clone()).collect::<Vec<_>>();

    assert_eq!(results.len(), 28);
    assert_eq!(hints.len(), 28);
    assert!(hints.iter().any(|hint| hint.chars().count() > 1));
  }

  #[test]
  fn match_docker() {
    let lines = split("latest sha256:30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4 20 hours ago");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text,
      "30557a29d5abc51e5f1d5b472e79b7e296f595abcf19fe6b9199dbbc809c6ff4"
    );
  }

  #[test]
  fn match_bash() {
    let lines = split("path: [32m/var/log/nginx.log[m\npath: [32mtest/log/nginx-2.log:32[mfolder/.nginx@4df2.log");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "/var/log/nginx.log");
    assert_eq!(results.get(1).unwrap().text, "test/log/nginx-2.log");
    assert_eq!(results.get(2).unwrap().text, "folder/.nginx@4df2.log");
  }

  #[test]
  fn match_paths() {
    let lines = split("Lorem /tmp/foo/bar_lol, lorem\n Lorem /var/log/boot-strap.log lorem ../log/kern.log lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "/tmp/foo/bar_lol");
    assert_eq!(results.get(1).unwrap().text.clone(), "/var/log/boot-strap.log");
    assert_eq!(results.get(2).unwrap().text.clone(), "../log/kern.log");
  }

  #[test]
  fn match_ls_columns_after_command() {
    let lines = split("~/repo ❯ ls\nCargo.toml  README.md  src  target\n~/repo ❯ echo done");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text, "Cargo.toml");
    assert_eq!(results.get(1).unwrap().text, "README.md");
    assert_eq!(results.get(2).unwrap().text, "src");
    assert_eq!(results.get(3).unwrap().text, "target");
  }

  #[test]
  fn match_ls_long_after_command() {
    let lines = split("~/repo ❯ ls -la\ntotal 16\ndrwxr-xr-x  12 me  staff   384 May 20 10:00 .git\n-rw-r--r--   1 me  staff  1024 May 20 10:00 README.md\nlrwxr-xr-x   1 me  staff     3 May 20 10:00 link -> src");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, ".git");
    assert_eq!(results.get(1).unwrap().text, "README.md");
    assert_eq!(results.get(2).unwrap().text, "link");
  }

  #[test]
  fn match_eza_output_after_command() {
    let lines = split("~/repo ❯ eza\nCargo.toml  README.md  src\n~/repo ❯ eza -la\n.rw-r--r-- 1.0k me 20 May 10:00 Cargo.toml");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text, "Cargo.toml");
    assert_eq!(results.get(1).unwrap().text, "README.md");
    assert_eq!(results.get(2).unwrap().text, "src");
    assert_eq!(results.get(3).unwrap().text, "Cargo.toml");
  }

  #[test]
  fn match_eza_icons_output_after_command() {
    let lines = split("~/repo ❯ eza --icons\n\u{e68b} Cargo.toml\n\u{f0ba} README.md\n\u{f31e} src");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "Cargo.toml");
    assert_eq!(results.get(0).unwrap().x, 4);
    assert_eq!(results.get(1).unwrap().text, "README.md");
    assert_eq!(results.get(1).unwrap().x, 4);
    assert_eq!(results.get(2).unwrap().text, "src");
    assert_eq!(results.get(2).unwrap().x, 4);
  }

  #[test]
  fn match_git_status_same_directory_files() {
    let lines = split("~/repo ❯ git status\nOn branch main\nChanges not staged for commit:\n  (use \"git add <file>...\" to update what will be committed)\n\tmodified:   Cargo.toml\n\tmodified:   README.md\n\nUntracked files:\n  (use \"git add <file>...\" to include in what will be committed)\n\tsrc.rs\n\nno changes added to commit (use \"git add\" and/or \"git commit -a\")");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text, "Cargo.toml");
    assert_eq!(results.get(1).unwrap().text, "README.md");
    assert_eq!(results.get(2).unwrap().text, "src.rs");
  }

  #[test]
  fn does_not_match_bare_words_outside_listing_output() {
    let lines = split("Cargo.toml README.md src target");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 0);
  }

  #[test]
  fn match_routes() {
    let lines = split("Lorem /app/routes/$routeId/$objectId, lorem\n Lorem /app/routes/$sectionId");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "/app/routes/$routeId/$objectId");
    assert_eq!(results.get(1).unwrap().text.clone(), "/app/routes/$sectionId");
  }

  #[test]
  fn match_home() {
    let lines = split("Lorem ~/.gnu/.config.txt, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "~/.gnu/.config.txt");
  }

  #[test]
  fn match_slugs() {
    let lines = split("Lorem dev/api/[slug]/foo, lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "dev/api/[slug]/foo");
  }

  #[test]
  fn match_uids() {
    let lines = split("Lorem ipsum 123e4567-e89b-12d3-a456-426655440000 lorem\n Lorem lorem lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
  }

  #[test]
  fn match_shas() {
    let lines = split("Lorem fd70b5695 5246ddf f924213 lorem\n Lorem 973113963b491874ab2e372ee60d4b4cb75f717c lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "f924213");
    assert_eq!(
      results.get(3).unwrap().text.clone(),
      "973113963b491874ab2e372ee60d4b4cb75f717c"
    );
  }

  #[test]
  fn match_ips() {
    let lines = split("Lorem ipsum 127.0.0.1 lorem\n Lorem 255.255.10.255 lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "127.0.0.1");
    assert_eq!(results.get(1).unwrap().text.clone(), "255.255.10.255");
    assert_eq!(results.get(2).unwrap().text.clone(), "127.0.0.1");
  }

  #[test]
  fn match_ipv6s() {
    let lines = split("Lorem ipsum fe80::2:202:fe4 lorem\n Lorem 2001:67c:670:202:7ba8:5e41:1591:d723 lorem fe80::2:1 lorem ipsum fe80:22:312:fe::1%eth0");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "fe80::2:202:fe4");
    assert_eq!(
      results.get(1).unwrap().text.clone(),
      "2001:67c:670:202:7ba8:5e41:1591:d723"
    );
    assert_eq!(results.get(2).unwrap().text.clone(), "fe80::2:1");
    assert_eq!(results.get(3).unwrap().text.clone(), "fe80:22:312:fe::1%eth0");
  }

  #[test]
  fn match_markdown_urls() {
    let lines = split("Lorem ipsum [link](https://github.io?foo=bar) ![](http://cdn.com/img.jpg) lorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(0).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "markdown_url");
    assert_eq!(results.get(1).unwrap().text.clone(), "http://cdn.com/img.jpg");
  }

  #[test]
  fn match_urls() {
    let lines = split("Lorem ipsum https://www.rust-lang.org/tools lorem\n Lorem ipsumhttps://crates.io lorem https://github.io?foo=bar lorem ssh://github.io");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "https://www.rust-lang.org/tools");
    assert_eq!(results.get(0).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(1).unwrap().text.clone(), "https://crates.io");
    assert_eq!(results.get(1).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(2).unwrap().text.clone(), "https://github.io?foo=bar");
    assert_eq!(results.get(2).unwrap().pattern.clone(), "url");
    assert_eq!(results.get(3).unwrap().text.clone(), "ssh://github.io");
    assert_eq!(results.get(3).unwrap().pattern.clone(), "url");
  }

  #[test]
  fn match_addresses() {
    let lines = split("Lorem 0xfd70b5695 0x5246ddf lorem\n Lorem 0x973113tlorem");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().text.clone(), "0xfd70b5695");
    assert_eq!(results.get(1).unwrap().text.clone(), "0x5246ddf");
    assert_eq!(results.get(2).unwrap().text.clone(), "0x973113");
  }

  #[test]
  fn match_hex_colors() {
    let lines = split("Lorem #fd7b56 lorem #FF00FF\n Lorem #00fF05 lorem #abcd00 lorem #afRR00");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 4);
    assert_eq!(results.get(0).unwrap().text.clone(), "#fd7b56");
    assert_eq!(results.get(1).unwrap().text.clone(), "#FF00FF");
    assert_eq!(results.get(2).unwrap().text.clone(), "#00fF05");
    assert_eq!(results.get(3).unwrap().text.clone(), "#abcd00");
  }

  #[test]
  fn match_ipfs() {
    let lines = split("Lorem QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ lorem Qmfoobar");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(
      results.get(0).unwrap().text.clone(),
      "QmRdbNSxDJBXmssAc9fvTtux4duptMvfSGiGuq6yHAQVKQ"
    );
  }

  #[test]
  fn match_process_port() {
    let lines =
      split("Lorem 5695 52463 lorem\n Lorem 973113 lorem 99999 lorem 8888 lorem\n   23456 lorem 5432 lorem 23444");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 8);
  }

  #[test]
  fn match_diff_a() {
    let lines = split("Lorem lorem\n--- a/src/main.rs");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_b() {
    let lines = split("Lorem lorem\n+++ b/src/main.rs");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 1);
    assert_eq!(results.get(0).unwrap().text.clone(), "src/main.rs");
  }

  #[test]
  fn match_diff_summary() {
    let lines = split("diff --git a/samples/test1 b/samples/test2");
    let custom = [].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 2);
    assert_eq!(results.get(0).unwrap().text.clone(), "samples/test1");
    assert_eq!(results.get(1).unwrap().text.clone(), "samples/test2");
  }

  #[test]
  fn priority() {
    let lines = split("Lorem [link](http://foo.bar) ipsum CUSTOM-52463 lorem ISSUE-123 lorem\nLorem /var/fd70b569/9999.log 52463 lorem\n Lorem 973113 lorem 123e4567-e89b-12d3-a456-426655440000 lorem 8888 lorem\n  https://crates.io/23456/fd70b569 lorem");
    let custom = ["CUSTOM-[0-9]{4,}", "ISSUE-[0-9]{3}"].to_vec();
    let results = State::new(&lines, "abcd", &custom).matches(false, false);

    assert_eq!(results.len(), 9);
    assert_eq!(results.get(0).unwrap().text.clone(), "http://foo.bar");
    assert_eq!(results.get(1).unwrap().text.clone(), "CUSTOM-52463");
    assert_eq!(results.get(2).unwrap().text.clone(), "ISSUE-123");
    assert_eq!(results.get(3).unwrap().text.clone(), "/var/fd70b569/9999.log");
    assert_eq!(results.get(4).unwrap().text.clone(), "52463");
    assert_eq!(results.get(5).unwrap().text.clone(), "973113");
    assert_eq!(
      results.get(6).unwrap().text.clone(),
      "123e4567-e89b-12d3-a456-426655440000"
    );
    assert_eq!(results.get(7).unwrap().text.clone(), "8888");
    assert_eq!(results.get(8).unwrap().text.clone(), "https://crates.io/23456/fd70b569");
  }
}
