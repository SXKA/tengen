//! A Piskvork brain running as a child process.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use tengen_engine::{Board, Color, Pos, Rule};

/// Why a brain failed to answer.
#[derive(Debug)]
pub enum BrainError {
    Spawn(std::io::Error),
    Exited,
    Timeout,
    Protocol(String),
}

impl std::fmt::Display for BrainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrainError::Spawn(e) => write!(f, "cannot start: {e}"),
            BrainError::Exited => write!(f, "exited"),
            BrainError::Timeout => write!(f, "timed out"),
            BrainError::Protocol(line) => write!(f, "unexpected reply {line:?}"),
        }
    }
}

pub struct Brain {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    timeout: Duration,
    /// `MESSAGE` lines received since the last [`Brain::take_messages`].
    messages: Vec<String>,
}

impl Brain {
    /// Start a brain and begin a game on a `size` board. `command` is the
    /// program followed by its arguments, separated by spaces.
    pub fn spawn(
        command: &str,
        size: usize,
        rule: Rule,
        turn_ms: u64,
        match_ms: u64,
        memory_mb: u64,
    ) -> Result<Brain, BrainError> {
        let mut words = command.split_whitespace();
        let program = words.next().unwrap_or_default();
        let mut child = Command::new(program)
            .args(words)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(BrainError::Spawn)?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (sender, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut brain = Brain {
            child,
            stdin,
            lines,
            timeout: Duration::from_millis(turn_ms * 3 + 2000),
            messages: Vec::new(),
        };
        // One thread and a modest table so many games can run side by side.
        brain.send("INFO thread_num 1");
        brain.send(&format!("INFO max_memory {}", memory_mb << 20));
        brain.send(&format!("INFO timeout_turn {turn_ms}"));
        brain.send(&format!("INFO timeout_match {match_ms}"));
        brain.send(&format!("INFO rule {}", rule.piskvork()));
        brain.send(&format!("START {size}"));
        match brain.read_reply(Duration::from_secs(10))? {
            line if line.trim() == "OK" => Ok(brain),
            line => Err(BrainError::Protocol(line)),
        }
    }

    /// Send a raw command line.
    pub fn send(&mut self, line: &str) {
        let _ = writeln!(self.stdin, "{line}");
        let _ = self.stdin.flush();
    }

    /// Tell the brain the opponent's move and read its answer.
    pub fn turn(&mut self, pos: Pos) -> Result<Pos, BrainError> {
        self.send(&format!("TURN {},{}", pos.x(), pos.y()));
        self.read_move()
    }

    /// Send the whole position, with the brain playing `color`, and read its
    /// answer.
    pub fn board(&mut self, board: &Board, color: Color) -> Result<Pos, BrainError> {
        if board.ply() == 0 {
            self.send("BEGIN");
            return self.read_move();
        }
        let mut text = String::from("BOARD\n");
        for (i, pos) in board.moves().enumerate() {
            let owner = if i % 2 == 0 { Color::Black } else { Color::White };
            let field = if owner == color { 1 } else { 2 };
            text.push_str(&format!("{},{},{field}\n", pos.x(), pos.y()));
        }
        text.push_str("DONE");
        self.send(&text);
        self.read_move()
    }

    /// The `MESSAGE` lines received since the last call.
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// End the game and the process.
    pub fn quit(&mut self) {
        self.send("END");
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
    }

    fn read_reply(&mut self, timeout: Duration) -> Result<String, BrainError> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    let word = line.split_whitespace().next().unwrap_or("");
                    if word == "MESSAGE" {
                        // Games never read them: keep only the latest.
                        if self.messages.len() >= 256 {
                            self.messages.drain(..128);
                        }
                        self.messages.push(line);
                        continue;
                    }
                    if matches!(word, "DEBUG" | "") {
                        continue;
                    }
                    return Ok(line);
                }
                Err(RecvTimeoutError::Timeout) => return Err(BrainError::Timeout),
                Err(RecvTimeoutError::Disconnected) => return Err(BrainError::Exited),
            }
        }
    }

    fn read_move(&mut self) -> Result<Pos, BrainError> {
        let line = self.read_reply(self.timeout)?;
        let parsed = line.trim().split_once(',').and_then(|(x, y)| {
            let (x, y) = (x.trim().parse::<usize>().ok()?, y.trim().parse::<usize>().ok()?);
            (x < tengen_engine::board::MAX_SIZE && y < tengen_engine::board::MAX_SIZE).then(|| Pos::new(x, y))
        });
        parsed.ok_or(BrainError::Protocol(line))
    }
}

impl Drop for Brain {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
