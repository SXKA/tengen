//! Tengen as a Gomocup brain, speaking the Piskvork protocol on stdin/stdout.
//!
//! Every rule of the protocol is supported: freestyle, exactly five, renju,
//! caro with and without overlines, and continuous games, plus the
//! `SWAP2BOARD` opening.
//!
//! Also runs `pbrain-tengen bench [depth]` to print the benchmark node count.
//! `--weights FILE` replaces the built-in evaluation weights, and
//! `--params name=value,...` changes search parameters (both for tuning).

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use tengen_engine::board::MAX_SIZE;
use tengen_engine::search::mate_moves;
use tengen_engine::{Board, Engine, Info, Limits, Pos, Rule, SearchResult, bench};

const ABOUT: &str = concat!(
    "name=\"Tengen\", version=\"",
    env!("CARGO_PKG_VERSION"),
    "\", author=\"SXKA\", country=\"Taiwan\", www=\"https://github.com/SXKA/tengen\""
);

/// Command lines from stdin, with lines read ahead during a search queued
/// for later.
struct Input {
    queue: VecDeque<String>,
    receiver: Receiver<String>,
}

impl Input {
    /// The next line, or `None` at end of input.
    fn next(&mut self) -> Option<String> {
        self.queue.pop_front().or_else(|| self.receiver.recv().ok())
    }

    /// Wait briefly for a line and queue it. Returns the line's command word.
    fn peek_new(&mut self, timeout: Duration) -> Option<String> {
        match self.receiver.recv_timeout(timeout) {
            Ok(line) => {
                let word = line.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
                if word != "YXSTOP" {
                    self.queue.push_back(line);
                }
                Some(word)
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
        }
    }
}

/// Settings and state of one brain process.
struct Brain {
    engine: Engine,
    board: Option<Board>,
    /// Milliseconds per move; 0 means "as fast as possible".
    timeout_turn: f64,
    /// Milliseconds for the whole game; 0 means unlimited.
    timeout_match: f64,
    /// Milliseconds left in the game, when the manager says.
    time_left: Option<f64>,
    max_memory: u64,
    rule: Rule,
}

impl Brain {
    fn new() -> Brain {
        let mut engine = Engine::new(256);
        engine.set_threads(std::thread::available_parallelism().map_or(1, |n| n.get()));
        Brain {
            engine,
            board: None,
            timeout_turn: 5000.0,
            timeout_match: 0.0,
            time_left: None,
            max_memory: 0,
            rule: Rule::Freestyle,
        }
    }

    /// A new board of `size` for the current rule.
    fn new_board(&self, size: usize) -> Board {
        Board::with_rule(size, self.rule)
    }

    fn limits(&self, board: &Board) -> Limits {
        let empty = board.size() * board.size() - board.ply();
        let left = match (self.time_left, self.timeout_match) {
            (Some(t), _) => t,
            (None, m) if m > 0.0 => m,
            _ => f64::INFINITY,
        };
        let turn = if self.timeout_turn > 0.0 {
            self.timeout_turn
        } else {
            100.0
        };
        Limits::for_clock(left, turn, empty, board.ply())
    }

    fn apply_info(&mut self, key: &str, value: &str) {
        let number = value.trim().parse::<f64>().ok();
        match (key.to_ascii_lowercase().as_str(), number) {
            ("timeout_turn", Some(n)) => self.timeout_turn = n,
            ("timeout_match", Some(n)) => self.timeout_match = n,
            ("time_left", Some(n)) => self.time_left = Some(n),
            ("max_memory", Some(n)) => {
                self.max_memory = n as u64;
                if self.max_memory > 0 {
                    // Leave room for the board, threads and the process itself.
                    let mb = (self.max_memory >> 20).saturating_sub(32) / 2;
                    self.engine.set_hash(mb.clamp(1, 1024) as usize);
                }
            }
            ("rule", Some(n)) => {
                // The continuous-game bit needs nothing more than accepting
                // field 3 in BOARD.
                let rule = Rule::from_piskvork(n as u32);
                if rule != self.rule {
                    self.rule = rule;
                    self.engine.clear();
                    if let Some(board) = self.board.take() {
                        self.board = Some(with_rule(&board, rule));
                    }
                }
            }
            ("thread_num", Some(n)) => self.engine.set_threads(n as usize),
            _ => {}
        }
    }

    /// Search the current position, print the move and play it.
    fn think(&mut self, input: &mut Input) {
        let Some(board) = self.board.as_ref() else {
            println!("ERROR no game started");
            return;
        };
        if board.is_full() {
            println!("ERROR the board is full");
            return;
        }
        let limits = self.limits(board);
        let stop = self.engine.stop_handle();
        let engine = &mut self.engine;
        let result = std::thread::scope(|scope| {
            let search = scope.spawn(|| engine.search(board, &limits, report));
            // Watch for commands that end the search early; keep the rest.
            while !search.is_finished() {
                if let Some(word) = input.peek_new(Duration::from_millis(5))
                    && (word == "YXSTOP" || word == "END")
                {
                    stop.store(true, Ordering::Relaxed);
                }
            }
            search.join().expect("search thread panicked")
        });
        let Some(pos) = result.best_move else {
            println!("ERROR no move");
            return;
        };
        if let Some(board) = self.board.as_mut() {
            board.play(pos);
        }
        println!("{},{}", pos.x(), pos.y());
    }

    /// Handle one command. Returns false on END.
    fn handle(&mut self, line: &str, input: &mut Input) -> bool {
        let line = line.trim();
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match word.to_ascii_uppercase().as_str() {
            "" => {}
            "START" => match rest.trim().parse::<usize>() {
                Ok(size) if (5..=MAX_SIZE).contains(&size) => {
                    self.board = Some(self.new_board(size));
                    self.engine.clear();
                    println!("OK");
                }
                _ => println!("ERROR unsupported board size {}", rest.trim()),
            },
            "RECTSTART" => println!("ERROR rectangular boards are not supported"),
            "RESTART" => match self.board.as_ref().map(Board::size) {
                Some(size) => {
                    self.board = Some(self.new_board(size));
                    self.engine.clear();
                    println!("OK");
                }
                None => println!("ERROR no game started"),
            },
            "BEGIN" => self.think(input),
            "TURN" => match (parse_move(rest), self.board.as_mut()) {
                (Some(pos), Some(board)) if board.is_empty(pos) => {
                    board.play(pos);
                    self.think(input);
                }
                (_, None) => println!("ERROR no game started"),
                _ => println!("ERROR invalid move {}", rest.trim()),
            },
            "BOARD" => {
                let Some(stones) = read_stones(input) else { return false };
                let rule = self.rule;
                match self
                    .board
                    .as_ref()
                    .map(Board::size)
                    .map(|size| board_from(size, rule, &stones))
                {
                    Some(Ok(board)) => {
                        self.board = Some(board);
                        self.think(input);
                    }
                    Some(Err(e)) => println!("ERROR {e}"),
                    None => println!("ERROR no game started"),
                }
            }
            "SWAP2BOARD" => {
                let Some(stones) = read_stones(input) else { return false };
                self.swap2(&stones);
            }
            "TAKEBACK" => match (parse_move(rest), self.board.as_mut()) {
                (Some(pos), Some(board)) if board.last_move() == Some(pos) => {
                    board.undo();
                    println!("OK");
                }
                _ => println!("ERROR cannot take back {}", rest.trim()),
            },
            "INFO" => {
                let (key, value) = rest.trim().split_once(char::is_whitespace).unwrap_or((rest.trim(), ""));
                self.apply_info(key, value);
            }
            "ABOUT" => println!("{ABOUT}"),
            "END" => return false,
            "YXSTOP" => {}
            _ => println!("UNKNOWN {line}"),
        }
        true
    }
}

impl Brain {
    /// Answer `SWAP2BOARD` with the opening stones listed so far, in the
    /// order they were placed (Black, White, Black, ...).
    ///
    /// * No stones: propose three, the most balanced of a set of standard
    ///   shapes.
    /// * Three or five stones: take the side that searches as better: play
    ///   White's next move, or answer `SWAP` to take Black.
    fn swap2(&mut self, stones: &[(usize, usize, u8)]) {
        let Some(size) = self.board.as_ref().map(Board::size) else {
            println!("ERROR no game started");
            return;
        };
        let budget = self.swap2_budget();
        let mut board = self.new_board(size);
        for &(x, y, _) in stones {
            let pos = Pos::new(x, y);
            if x >= size || y >= size || !board.is_empty(pos) {
                println!("ERROR bad stone {x},{y}");
                return;
            }
            board.play(pos);
        }
        match stones.len() {
            0 => {
                let opening = self.balanced_opening(size, budget);
                let mut board = self.new_board(size);
                for &pos in &opening {
                    board.play(pos);
                }
                self.board = Some(board);
                let text: Vec<String> = opening.iter().map(|p| format!("{},{}", p.x(), p.y())).collect();
                println!("{}", text.join(" "));
            }
            3 | 5 => {
                // White is to move; its score decides who we want to be.
                let result = self.engine.search(&board, &Limits::movetime(budget), |_| {});
                match result.best_move {
                    Some(pos) if result.value >= 0 => {
                        board.play(pos);
                        self.board = Some(board);
                        println!("{},{}", pos.x(), pos.y());
                    }
                    _ => {
                        self.board = Some(board);
                        println!("SWAP");
                    }
                }
            }
            n => println!("ERROR SWAP2BOARD with {n} stones"),
        }
        self.engine.clear();
    }

    /// Milliseconds to spend on a swap2 decision.
    fn swap2_budget(&self) -> f64 {
        let turn = if self.timeout_turn > 0.0 {
            self.timeout_turn
        } else {
            1000.0
        };
        let left = match (self.time_left, self.timeout_match) {
            (Some(t), _) => t / 8.0,
            (None, m) if m > 0.0 => m / 8.0,
            _ => f64::INFINITY,
        };
        (turn.min(left) * 0.8 - 50.0).max(20.0)
    }

    /// Three opening stones around the center, chosen so that the side to
    /// move (White) is as close to even as a short search can tell.
    fn balanced_opening(&mut self, size: usize, budget: f64) -> Vec<Pos> {
        let c = (size / 2) as isize;
        let at = |dx: isize, dy: isize| Pos::new((c + dx) as usize, (c + dy) as usize);
        let mut candidates = Vec::new();
        for (wx, wy) in [(1, 0), (1, 1)] {
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if (dx, dy) != (0, 0) && (dx, dy) != (wx, wy) {
                        candidates.push([at(0, 0), at(wx, wy), at(dx, dy)]);
                    }
                }
            }
        }
        let each = (budget / candidates.len() as f64).max(5.0);
        let mut best = (i32::MAX, candidates[0]);
        for stones in candidates {
            let mut board = self.new_board(size);
            for &pos in &stones {
                board.play(pos);
            }
            let result: SearchResult = self.engine.search(&board, &Limits::movetime(each), |_| {});
            if result.value.abs() < best.0 {
                best = (result.value.abs(), stones);
            }
        }
        best.1.to_vec()
    }
}

/// Read `x,y[,field]` lines up to `DONE`. `None` at end of input.
fn read_stones(input: &mut Input) -> Option<Vec<(usize, usize, u8)>> {
    let mut stones = Vec::new();
    loop {
        let entry = input.next()?;
        let entry = entry.trim();
        if entry.eq_ignore_ascii_case("DONE") {
            return Some(stones);
        }
        let fields: Vec<&str> = entry.split(',').map(str::trim).collect();
        match fields[..] {
            [x, y, who] => {
                if let (Ok(x), Ok(y), Ok(who)) = (x.parse(), y.parse(), who.parse::<u8>()) {
                    stones.push((x, y, who));
                }
            }
            [x, y] => {
                if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                    stones.push((x, y, 0));
                }
            }
            _ => {}
        }
    }
}

/// The same position under another rule.
fn with_rule(board: &Board, rule: Rule) -> Board {
    let mut out = Board::with_rule(board.size(), rule);
    for pos in board.walls() {
        out.place_wall(pos);
    }
    for pos in board.moves() {
        out.play(pos);
    }
    out
}

fn report(info: &Info) {
    let pv: Vec<String> = info.pv.iter().map(|&p| coordinate(p)).collect();
    let score = match mate_moves(info.value) {
        Some(m) => format!("mate {m}"),
        None => format!("ev {}", info.value),
    };
    println!(
        "MESSAGE depth {}-{} {} n {} nps {} tm {:.0} pv {}",
        info.depth,
        info.seldepth,
        score,
        info.nodes,
        info.nps(),
        info.elapsed_ms,
        pv.join(" ")
    );
}

/// `h8`-style name of a cell: column letter, row number from 1.
fn coordinate(pos: Pos) -> String {
    format!("{}{}", (b'a' + pos.x() as u8) as char, pos.y() + 1)
}

fn parse_move(text: &str) -> Option<Pos> {
    let (x, y) = text.trim().split_once(',')?;
    let (x, y) = (x.trim().parse::<usize>().ok()?, y.trim().parse::<usize>().ok()?);
    (x < MAX_SIZE && y < MAX_SIZE).then(|| Pos::new(x, y))
}

/// Rebuild a position from a BOARD listing: field 1 is our stone, 2 the
/// opponent's, in the order they were played, and 3 (continuous games) a
/// stone that belongs to nobody any more. We are always to move.
fn board_from(size: usize, rule: Rule, stones: &[(usize, usize, u8)]) -> Result<Board, String> {
    if let Some(&(x, y, _)) = stones.iter().find(|s| s.0 >= size || s.1 >= size) {
        return Err(format!("stone {x},{y} is off the board"));
    }
    let ours: Vec<Pos> = stones.iter().filter(|s| s.2 == 1).map(|s| Pos::new(s.0, s.1)).collect();
    let theirs: Vec<Pos> = stones.iter().filter(|s| s.2 == 2).map(|s| Pos::new(s.0, s.1)).collect();
    let walls: Vec<Pos> = stones.iter().filter(|s| s.2 == 3).map(|s| Pos::new(s.0, s.1)).collect();
    let (first, second) = match theirs.len().checked_sub(ours.len()) {
        Some(0) => (&ours, &theirs),
        Some(1) => (&theirs, &ours),
        _ => {
            return Err(format!(
                "{} own and {} opponent stones cannot be our turn",
                ours.len(),
                theirs.len()
            ));
        }
    };
    let mut board = Board::with_rule(size, rule);
    for &pos in &walls {
        if !board.is_empty(pos) {
            return Err(format!("bad stone {},{}", pos.x(), pos.y()));
        }
        board.place_wall(pos);
    }
    for i in 0..first.len() {
        for pos in [first.get(i), second.get(i)].into_iter().flatten() {
            if !board.is_empty(*pos) {
                return Err(format!("bad stone {},{}", pos.x(), pos.y()));
            }
            board.play(*pos);
        }
    }
    Ok(board)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(i) = args.iter().position(|a| a == "--weights") {
        let path = args.get(i + 1).cloned().unwrap_or_default();
        let weights = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| tengen_engine::eval::Weights::parse(&text));
        match weights.and_then(|w| tengen_engine::eval::install(&w)) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("--weights {path}: {e}");
                std::process::exit(2);
            }
        }
        args.drain(i..(i + 2).min(args.len()));
    }
    if let Some(i) = args.iter().position(|a| a == "--params") {
        let text = args.get(i + 1).cloned().unwrap_or_default();
        let params = tengen_engine::search::Params::parse(&text);
        if let Err(e) = params.and_then(tengen_engine::search::set_params) {
            eprintln!("--params {text}: {e}");
            std::process::exit(2);
        }
        args.drain(i..(i + 2).min(args.len()));
    }
    match args.first().map(String::as_str) {
        Some("bench") => {
            let depth = args.get(1).and_then(|d| d.parse().ok()).unwrap_or(7);
            let result = bench::run(depth, tengen_engine::search::default_clock);
            println!(
                "nodes {}\nnps {}\ntime {:.0} ms",
                result.nodes,
                result.nps(),
                result.elapsed_ms
            );
            return;
        }
        Some("--version" | "-V") => {
            println!("tengen {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {}
    }

    let (sender, commands) = mpsc::channel();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let mut brain = Brain::new();
    let mut input = Input {
        queue: VecDeque::new(),
        receiver: commands,
    };
    while let Some(line) = input.next() {
        let keep_going = brain.handle(&line, &mut input);
        let _ = io::stdout().flush();
        if !keep_going {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_positions_for_either_color() {
        let board_from = |size, stones: &[(usize, usize, u8)]| board_from(size, Rule::Freestyle, stones);
        let board = board_from(15, &[(7, 7, 2), (8, 8, 1), (7, 8, 2)]).unwrap();
        assert_eq!(board.ply(), 3);
        assert_eq!(
            board.moves().collect::<Vec<_>>(),
            [Pos::new(7, 7), Pos::new(8, 8), Pos::new(7, 8)]
        );
        let board = board_from(15, &[(7, 7, 1), (8, 8, 2)]).unwrap();
        assert_eq!(board.moves().next(), Some(Pos::new(7, 7)));
        assert!(board_from(15, &[(7, 7, 1)]).is_err());
        assert!(board_from(15, &[(7, 7, 1), (7, 7, 2)]).is_err());
        assert_eq!(board_from(15, &[]).unwrap().ply(), 0);
        assert!(board_from(15, &[(15, 0, 1), (0, 0, 2)]).is_err());
    }

    /// Run the brain on a script of commands and collect what it prints
    /// (moves and answers, not MESSAGE lines).
    fn session(brain: &mut Brain, script: &str) -> Vec<String> {
        let (sender, receiver) = mpsc::channel();
        for line in script.lines() {
            sender.send(line.to_string()).unwrap();
        }
        drop(sender);
        let mut input = Input {
            queue: VecDeque::new(),
            receiver,
        };
        let mut out = Vec::new();
        while let Some(line) = input.next() {
            let before = brain.board.as_ref().map(|b| b.moves().collect::<Vec<_>>());
            brain.handle(&line, &mut input);
            let after = brain.board.as_ref().map(|b| b.moves().collect::<Vec<_>>());
            if before != after {
                out.push(format!("{:?}", after.and_then(|m| m.last().copied())));
            }
        }
        out
    }

    #[test]
    fn renju_rule_and_continuous_boards() {
        let mut brain = Brain::new();
        brain.engine.set_threads(1);
        brain.apply_info("timeout_turn", "200");
        brain.apply_info("rule", "6");
        assert_eq!(brain.rule, Rule::Renju);
        // We are Black (one stone each, our turn). (8, 7) is our double
        // three, so it must not be played; (3, 3) is a wall.
        let script = "START 15\nBOARD\n6,7,1\n0,0,2\n7,7,1\n0,14,2\n8,5,1\n14,0,2\n8,6,1\n14,14,2\n3,3,3\nDONE";
        session(&mut brain, script);
        let board = brain.board.as_mut().unwrap();
        assert_eq!(board.rule(), Rule::Renju);
        assert!(board.is_wall(Pos::new(3, 3)));
        let last = board.last_move().unwrap();
        assert_ne!(last, Pos::new(8, 7));
        assert_eq!(board.stone(last), Some(tengen_engine::Color::Black));
        // A rule change keeps the position.
        brain.apply_info("rule", "1");
        let board = brain.board.as_ref().unwrap();
        assert_eq!((board.rule(), board.ply()), (Rule::Standard, 9));
        assert!(board.is_wall(Pos::new(3, 3)));
    }

    #[test]
    fn swap2_proposes_and_chooses() {
        let mut brain = Brain::new();
        brain.engine.set_threads(1);
        brain.apply_info("timeout_turn", "300");
        session(&mut brain, "START 15\nSWAP2BOARD\nDONE");
        assert_eq!(brain.board.as_ref().unwrap().ply(), 3);
        // A clearly winning Black opening: we would rather be Black.
        session(&mut brain, "SWAP2BOARD\n7,7\n0,0\n8,8\nDONE");
        assert_eq!(brain.board.as_ref().unwrap().ply(), 3);
        // A hopeless one for Black: keep White and play.
        session(&mut brain, "SWAP2BOARD\n0,0\n7,7\n14,14\n8,8\n0,14\nDONE");
        assert_eq!(brain.board.as_ref().unwrap().ply(), 6);
    }

    #[test]
    fn parses_moves_and_coordinates() {
        assert_eq!(parse_move(" 3, 12"), Some(Pos::new(3, 12)));
        assert_eq!(parse_move("3"), None);
        assert_eq!(parse_move("25,1"), None);
        assert_eq!(coordinate(Pos::new(7, 7)), "h8");
    }

    #[test]
    fn budgets_time_from_the_clock() {
        let mut brain = Brain::new();
        let board = Board::new(15);
        brain.apply_info("timeout_turn", "1000");
        let l = brain.limits(&board);
        assert!(l.hard_ms.unwrap() <= 1000.0 && l.hard_ms.unwrap() > 500.0);
        // No game clock: most of the turn is used.
        assert!(l.soft_ms.unwrap() > 800.0);
        brain.apply_info("time_left", "3000");
        let l = brain.limits(&board);
        assert!(l.hard_ms.unwrap() < 500.0 && l.soft_ms.unwrap() < 0.5 * l.hard_ms.unwrap());
    }
}
