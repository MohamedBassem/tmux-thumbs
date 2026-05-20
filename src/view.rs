use super::*;
use std::char;
use std::fmt::Write as FmtWrite;
use std::io::{stdout, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::process::Command;
use termion::async_stdin;
use termion::clear;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::screen::AlternateScreen;
use termion::{color, cursor};

use unicode_width::UnicodeWidthStr;

pub struct View<'a> {
  state: &'a mut state::State<'a>,
  skip: usize,
  multi: bool,
  contrast: bool,
  position: &'a str,
  matches: Vec<state::Match<'a>>,
  select_foreground_color: Box<dyn color::Color>,
  select_background_color: Box<dyn color::Color>,
  multi_foreground_color: Box<dyn color::Color>,
  multi_background_color: Box<dyn color::Color>,
  foreground_color: Box<dyn color::Color>,
  background_color: Box<dyn color::Color>,
  hint_background_color: Box<dyn color::Color>,
  hint_foreground_color: Box<dyn color::Color>,
  ready_signal: Option<&'a str>,
  ready_sent: bool,
  pane_width: Option<usize>,
  chosen: Vec<(String, bool)>,
}

enum CaptureEvent {
  Exit,
  Hint,
}

enum InputEvent {
  Backspace,
  Down,
  Enter,
  Esc,
  Hint(String),
  Left,
  Right,
  Space,
  Up,
}

impl<'a> View<'a> {
  pub fn new(
    state: &'a mut state::State<'a>,
    multi: bool,
    reverse: bool,
    unique: bool,
    contrast: bool,
    position: &'a str,
    select_foreground_color: Box<dyn color::Color>,
    select_background_color: Box<dyn color::Color>,
    multi_foreground_color: Box<dyn color::Color>,
    multi_background_color: Box<dyn color::Color>,
    foreground_color: Box<dyn color::Color>,
    background_color: Box<dyn color::Color>,
    hint_foreground_color: Box<dyn color::Color>,
    hint_background_color: Box<dyn color::Color>,
    ready_signal: Option<&'a str>,
    pane_width: Option<usize>,
  ) -> View<'a> {
    let matches = state.matches(reverse, unique);
    let skip = if reverse && !matches.is_empty() { matches.len() - 1 } else { 0 };

    View {
      state,
      skip,
      multi,
      contrast,
      position,
      matches,
      select_foreground_color,
      select_background_color,
      multi_foreground_color,
      multi_background_color,
      foreground_color,
      background_color,
      hint_foreground_color,
      hint_background_color,
      ready_signal,
      ready_sent: false,
      pane_width,
      chosen: vec![],
    }
  }

  pub fn prev(&mut self) {
    if self.skip > 0 {
      self.skip -= 1;
    }
  }

  pub fn next(&mut self) {
    if self.skip < self.matches.len() - 1 {
      self.skip += 1;
    }
  }

  fn make_hint_text(&self, hint: &str) -> String {
    if self.contrast {
      format!("[{}]", hint)
    } else {
      hint.to_string()
    }
  }

  fn render(&self, stdout: &mut dyn Write, typed_hint: &str) -> () {
    let mut frame = String::new();
    let line_offsets = self.line_offsets();

    write!(&mut frame, "{}{}{}", cursor::Hide, cursor::Goto(1, 1), clear::All).unwrap();

    for (index, line) in self.state.lines.iter().enumerate() {
      for (row, text) in self.display_rows(line).iter().enumerate() {
        let clean = text.trim_end_matches(|c: char| c.is_whitespace());

        if !clean.is_empty() {
          write!(
            &mut frame,
            "{goto}{text}",
            goto = cursor::Goto(1, (line_offsets[index] + row) as u16 + 1),
            text = text
          )
          .unwrap();
        }
      }
    }

    let selected = self.matches.get(self.skip);

    for mat in self.matches.iter() {
      let chosen_hint = self.chosen.iter().any(|(hint, _)| hint == mat.text);

      let selected_color = if chosen_hint {
        &self.multi_foreground_color
      } else if selected == Some(mat) {
        &self.select_foreground_color
      } else {
        &self.foreground_color
      };
      let selected_background_color = if chosen_hint {
        &self.multi_background_color
      } else if selected == Some(mat) {
        &self.select_background_color
      } else {
        &self.background_color
      };

      // Find long utf sequences and extract it from mat.x
      let line = &self.state.lines[mat.y as usize];
      let offset = self.visual_column_at(line, mat.x as usize);
      let text = self.make_hint_text(mat.text);
      let (match_x, match_y) = self.translate_position(line_offsets[mat.y as usize], offset);

      write!(
        &mut frame,
        "{goto}{background}{foregroud}{text}{resetf}{resetb}",
        goto = cursor::Goto(match_x as u16 + 1, match_y as u16 + 1),
        foregroud = color::Fg(&**selected_color),
        background = color::Bg(&**selected_background_color),
        resetf = color::Fg(color::Reset),
        resetb = color::Bg(color::Reset),
        text = &text
      )
      .unwrap();

      if let Some(ref hint) = mat.hint {
        let extra_position = match self.position {
          "right" => text.width_cjk() - hint.len(),
          "off_left" => 0 - hint.len() - if self.contrast { 2 } else { 0 },
          "off_right" => text.width_cjk(),
          _ => 0,
        };

        let text = self.make_hint_text(hint.as_str());
        let final_position = std::cmp::max(offset as i16 + extra_position as i16, 0) as usize;
        let (hint_x, hint_y) = self.translate_position(line_offsets[mat.y as usize], final_position);

        write!(
          &mut frame,
          "{goto}{background}{foregroud}{text}{resetf}{resetb}",
          goto = cursor::Goto(hint_x as u16 + 1, hint_y as u16 + 1),
          foregroud = color::Fg(&*self.hint_foreground_color),
          background = color::Bg(&*self.hint_background_color),
          resetf = color::Fg(color::Reset),
          resetb = color::Bg(color::Reset),
          text = &text
        )
        .unwrap();

        if hint.starts_with(typed_hint) {
          write!(
            &mut frame,
            "{goto}{background}{foregroud}{text}{resetf}{resetb}",
            goto = cursor::Goto(hint_x as u16 + 1, hint_y as u16 + 1),
            foregroud = color::Fg(&*self.multi_foreground_color),
            background = color::Bg(&*self.multi_background_color),
            resetf = color::Fg(color::Reset),
            resetb = color::Bg(color::Reset),
            text = &typed_hint
          )
          .unwrap();
        }
      }
    }

    stdout.write_all(frame.as_bytes()).unwrap();
    stdout.flush().unwrap();
  }

  fn display_rows(&self, line: &str) -> Vec<String> {
    let width = match self.pane_width {
      Some(width) if width > 0 => width,
      _ => return vec![line.to_string()],
    };

    let mut rows = Vec::new();
    let mut row = String::new();
    let mut row_width = 0;

    for ch in line.chars() {
      let ch_width = Self::char_width_at(ch, row_width);

      if row_width > 0 && row_width + ch_width > width {
        rows.push(row);
        row = String::new();
        row_width = 0;
      }

      row.push(ch);
      row_width += ch_width;
    }

    rows.push(row);
    rows
  }

  fn line_offsets(&self) -> Vec<usize> {
    let mut offsets = Vec::new();
    let mut offset = 0;

    for line in self.state.lines.iter() {
      offsets.push(offset);
      offset += self.display_rows(line).len();
    }

    offsets
  }

  fn translate_position(&self, line_offset: usize, column: usize) -> (usize, usize) {
    if let Some(width) = self.pane_width {
      if width > 0 {
        return (column % width, line_offset + (column / width));
      }
    }

    (column, line_offset)
  }

  fn visual_column_at(&self, line: &str, byte_index: usize) -> usize {
    let mut column = 0;

    for (index, ch) in line.char_indices() {
      if index >= byte_index {
        break;
      }

      column += Self::char_width_at(ch, column);
    }

    column
  }

  fn char_width_at(ch: char, column: usize) -> usize {
    if ch == '\t' {
      8 - (column % 8)
    } else if Self::is_private_use(ch) {
      1
    } else {
      ch.to_string().width_cjk().max(1)
    }
  }

  fn is_private_use(ch: char) -> bool {
    matches!(ch as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
  }

  fn signal_ready(&mut self) {
    if self.ready_sent {
      return;
    }

    self.ready_sent = true;

    if let Some(signal) = self.ready_signal {
      Command::new("tmux")
        .arg("wait-for")
        .arg("-S")
        .arg(signal)
        .output()
        .expect("Unable to signal ready state");
    }
  }

  fn process_event(&mut self, event: InputEvent, typed_hint: &mut String, longest_hint: &str) -> Option<CaptureEvent> {
    match event {
      InputEvent::Esc => {
        if self.multi && !typed_hint.is_empty() {
          typed_hint.clear();
        } else {
          return Some(CaptureEvent::Exit);
        }
      }
      InputEvent::Up | InputEvent::Left => {
        self.prev();
      }
      InputEvent::Down | InputEvent::Right => {
        self.next();
      }
      InputEvent::Backspace => {
        typed_hint.pop();
      }
      InputEvent::Enter => match self.matches.iter().enumerate().find(|&h| h.0 == self.skip) {
        Some(hm) => {
          self.chosen.push((hm.1.text.to_string(), false));

          if !self.multi {
            return Some(CaptureEvent::Hint);
          }
        }
        _ => panic!("Match not found?"),
      },
      InputEvent::Space => {
        if self.multi {
          return Some(CaptureEvent::Hint);
        } else {
          self.multi = true;
        }
      }
      InputEvent::Hint(key) => {
        let lower_key = key.to_lowercase();

        typed_hint.push_str(lower_key.as_str());

        let selection = self.matches.iter().find(|mat| mat.hint == Some(typed_hint.clone()));

        match selection {
          Some(mat) => {
            self.chosen.push((mat.text.to_string(), key != lower_key));

            if self.multi {
              typed_hint.clear();
            } else {
              return Some(CaptureEvent::Hint);
            }
          }
          None => {
            if !self.multi && typed_hint.len() >= longest_hint.len() {
              return Some(CaptureEvent::Exit);
            }
          }
        }
      }
    }

    None
  }

  fn longest_hint(&self) -> String {
    self
      .matches
      .iter()
      .filter_map(|m| m.hint.clone())
      .max_by(|x, y| x.len().cmp(&y.len()))
      .unwrap()
      .clone()
  }

  fn listen(&mut self, stdin: &mut dyn Read, stdout: &mut dyn Write) -> CaptureEvent {
    if self.matches.is_empty() {
      self.signal_ready();
      return CaptureEvent::Exit;
    }

    let mut typed_hint: String = "".to_owned();
    let longest_hint = self.longest_hint();

    self.render(stdout, &typed_hint);
    self.signal_ready();

    loop {
      let mut handled_event = |event| {
        match self.process_event(event, &mut typed_hint, &longest_hint) {
          Some(CaptureEvent::Hint) => Some(CaptureEvent::Hint),
          Some(CaptureEvent::Exit) => Some(CaptureEvent::Exit),
          None => None,
        }
      };

      match stdin.keys().next() {
        Some(key) => {
          match key {
            Ok(key) => {
              let result = match key {
                Key::Esc => handled_event(InputEvent::Esc),
                Key::Up => handled_event(InputEvent::Up),
                Key::Down => handled_event(InputEvent::Down),
                Key::Left => handled_event(InputEvent::Left),
                Key::Right => handled_event(InputEvent::Right),
                Key::Backspace => handled_event(InputEvent::Backspace),
                Key::Char(ch) => {
                  let event = match ch {
                    '\n' => InputEvent::Enter,
                    ' ' => InputEvent::Space,
                    key => InputEvent::Hint(key.to_string()),
                  };

                  handled_event(event)
                }
                _ => {
                  // Unknown key
                  None
                }
              };

              match result {
                Some(CaptureEvent::Hint) => return CaptureEvent::Hint,
                Some(CaptureEvent::Exit) => break,
                None => {}
              };
            }
            Err(err) => panic!("{}", err),
          }

          stdin.keys().for_each(|_| { /* Skip the rest of stdin buffer */ })
        }
        _ => {
          // Nothing in the buffer. Wait for a bit...
          std::thread::sleep(std::time::Duration::from_millis(50));
          continue; // don't render again if nothing new to show
        }
      }

      self.render(stdout, &typed_hint);
    }

    CaptureEvent::Exit
  }

  pub fn present(&mut self) -> Vec<(String, bool)> {
    let mut stdin = async_stdin();
    let mut stdout = AlternateScreen::from(stdout().into_raw_mode().unwrap());

    let hints = match self.listen(&mut stdin, &mut stdout) {
      CaptureEvent::Exit => vec![],
      CaptureEvent::Hint => self.chosen.clone(),
    };

    write!(stdout, "{}", cursor::Show).unwrap();

    hints
  }

  pub fn present_socket(&mut self, socket_path: &str) -> Vec<(String, bool)> {
    if self.matches.is_empty() {
      self.signal_ready();
      return vec![];
    }

    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path).expect("Unable to bind input socket");
    let mut stdout = stdout();
    let mut typed_hint: String = "".to_owned();
    let longest_hint = self.longest_hint();

    self.render(&mut stdout, &typed_hint);
    self.signal_ready();

    for stream in listener.incoming() {
      let stream = stream.expect("Unable to read input socket");
      let mut reader = BufReader::new(stream);
      let mut input = String::new();

      std::io::BufRead::read_line(&mut reader, &mut input).expect("Unable to read input");

      let event = match input.trim_end() {
        "backspace" => InputEvent::Backspace,
        "down" => InputEvent::Down,
        "enter" => InputEvent::Enter,
        "esc" => InputEvent::Esc,
        "left" => InputEvent::Left,
        "right" => InputEvent::Right,
        "space" => InputEvent::Space,
        "up" => InputEvent::Up,
        item if item.starts_with("hint:") => InputEvent::Hint(item.trim_start_matches("hint:").to_string()),
        _ => continue,
      };

      match self.process_event(event, &mut typed_hint, &longest_hint) {
        Some(CaptureEvent::Hint) => return self.chosen.clone(),
        Some(CaptureEvent::Exit) => return vec![],
        None => self.render(&mut stdout, &typed_hint),
      }
    }

    vec![]
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn split(output: &str) -> Vec<&str> {
    output.split("\n").collect::<Vec<&str>>()
  }

  #[test]
  fn hint_text() {
    let lines = split("lorem 127.0.0.1 lorem");
    let custom = [].to_vec();
    let mut state = state::State::new(&lines, "abcd", &custom);
    let mut view = View {
      state: &mut state,
      skip: 0,
      multi: false,
      contrast: false,
      position: &"",
      matches: vec![],
      select_foreground_color: colors::get_color("default"),
      select_background_color: colors::get_color("default"),
      multi_foreground_color: colors::get_color("default"),
      multi_background_color: colors::get_color("default"),
      foreground_color: colors::get_color("default"),
      background_color: colors::get_color("default"),
      hint_background_color: colors::get_color("default"),
      hint_foreground_color: colors::get_color("default"),
      ready_signal: None,
      ready_sent: false,
      pane_width: None,
      chosen: vec![],
    };

    let result = view.make_hint_text("a");
    assert_eq!(result, "a".to_string());

    view.contrast = true;
    let result = view.make_hint_text("a");
    assert_eq!(result, "[a]".to_string());
  }

  #[test]
  fn private_use_icons_use_reported_terminal_width() {
    assert_eq!(View::char_width_at('\u{e68b}', 0), 1);
    assert_eq!(View::char_width_at('a', 0), 1);
  }
}
