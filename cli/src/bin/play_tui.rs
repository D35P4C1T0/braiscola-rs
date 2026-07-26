use std::collections::VecDeque;
use std::io;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use briscola_ai::mc::BestMoveResult;
use briscola_core::card::{Card, HAND_SIZE};
use briscola_core::state::Player;
use cli::advisor::format_card;
use cli::card_art::{TerminalCardRenderer, card_name_english, card_name_italian};
use cli::play::{PlayConfig, PlayError, PlayableGame, winner_from_scores};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, LineGauge, List, ListItem, Paragraph, Row, Table, Wrap,
};
use ratatui::{Frame, Terminal};

mod theme {
    use ratatui::style::Color;

    pub const BACKGROUND: Color = Color::Rgb(7, 18, 16);
    pub const SURFACE: Color = Color::Rgb(11, 31, 27);
    pub const SURFACE_RAISED: Color = Color::Rgb(17, 43, 37);
    pub const BORDER: Color = Color::Rgb(52, 91, 78);
    pub const TEXT: Color = Color::Rgb(226, 232, 225);
    pub const MUTED: Color = Color::Rgb(125, 151, 140);
    pub const GOLD: Color = Color::Rgb(241, 191, 79);
    pub const GOLD_SOFT: Color = Color::Rgb(117, 83, 32);
    pub const PLAYER: Color = Color::Rgb(92, 211, 157);
    pub const OPPONENT: Color = Color::Rgb(238, 111, 100);
    pub const INFO: Color = Color::Rgb(102, 190, 220);
}

#[derive(Debug, Clone, Copy)]
struct CliOptions {
    seed: u64,
    hint_samples: usize,
    opponent_samples: usize,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self { seed: random_seed(), hint_samples: 128, opponent_samples: 96 }
    }
}

fn random_seed() -> u64 {
    let base = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            let nanos = duration.as_nanos();
            let low = u64::try_from(nanos & u128::from(u64::MAX)).unwrap_or(0);
            let high = u64::try_from(nanos >> 64).unwrap_or(0);
            low ^ high
        }
        Err(_) => 0x9E37_79B9_7F4A_7C15,
    };
    base ^ u64::from(std::process::id())
}

struct UiState {
    game: PlayableGame,
    seed: u64,
    selected_index: usize,
    hint_enabled: bool,
    cached_hint: Option<BestMoveResult>,
    status: String,
    log: VecDeque<String>,
    renderer: TerminalCardRenderer,
    table_renderer: TerminalCardRenderer,
    art_error: Option<String>,
    last_trick: Option<CompletedTrickView>,
    winner_flash_on: bool,
}

#[derive(Debug, Clone, Copy)]
struct CompletedTrickView {
    my_card: Card,
    opp_card: Card,
    winner: Player,
}

#[derive(Debug, Clone, Copy)]
struct TableSlotView<'a> {
    title: &'a str,
    card: Option<Card>,
    is_winner_highlighted: bool,
}

const HAND_SLOTS: usize = HAND_SIZE;

impl UiState {
    fn new(config: CliOptions) -> Result<Self, String> {
        let game = PlayableGame::new(PlayConfig {
            seed: config.seed,
            hint_samples: config.hint_samples,
            opponent_samples: config.opponent_samples,
        })
        .map_err(|error| format!("cannot initialize game: {error:?}"))?;

        let mut log = VecDeque::with_capacity(16);
        log.push_back(String::from("Welcome. You are Me."));

        Ok(Self {
            game,
            seed: config.seed,
            selected_index: 0,
            hint_enabled: false,
            cached_hint: None,
            status: String::from("Select a card and press Enter."),
            log,
            renderer: TerminalCardRenderer::new(14),
            table_renderer: TerminalCardRenderer::new(10),
            art_error: None,
            last_trick: None,
            winner_flash_on: false,
        })
    }

    fn push_log(&mut self, line: String) {
        while self.log.len() >= 14 {
            let _ = self.log.pop_front();
        }
        self.log.push_back(line);
    }
}

struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalGuard {
    fn new() -> Result<Self, String> {
        enable_raw_mode().map_err(|error| format!("raw mode enable failed: {error}"))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)
            .map_err(|error| format!("terminal init failed: {error}"))?;
        let backend = CrosstermBackend::new(stdout);
        let terminal =
            Terminal::new(backend).map_err(|error| format!("terminal error: {error}"))?;
        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut Terminal<CrosstermBackend<io::Stdout>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
    }
}

fn parse_cli_options() -> Result<CliOptions, String> {
    let mut options = CliOptions::default();
    let mut args = std::env::args().skip(1);
    let mut parsed_positional_seed = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" | "-s" => {
                let Some(value) = args.next() else {
                    return Err(String::from("missing value after --seed"));
                };
                options.seed = value
                    .parse::<u64>()
                    .map_err(|error| format!("invalid --seed value '{value}': {error}"))?;
            }
            "--hint-samples" => {
                let Some(value) = args.next() else {
                    return Err(String::from("missing value after --hint-samples"));
                };
                options.hint_samples = value
                    .parse::<usize>()
                    .map_err(|error| format!("invalid --hint-samples value '{value}': {error}"))?;
            }
            "--opponent-samples" => {
                let Some(value) = args.next() else {
                    return Err(String::from("missing value after --opponent-samples"));
                };
                options.opponent_samples = value.parse::<usize>().map_err(|error| {
                    format!("invalid --opponent-samples value '{value}': {error}")
                })?;
            }
            "-h" | "--help" => {
                return Err(String::from(
                    "usage: play_tui [seed] [--seed N|-s N] [--hint-samples N] [--opponent-samples N]\nseed defaults to a random value when omitted",
                ));
            }
            _ if !arg.starts_with('-') && !parsed_positional_seed => {
                options.seed = arg
                    .parse::<u64>()
                    .map_err(|error| format!("invalid positional seed '{arg}': {error}"))?;
                parsed_positional_seed = true;
            }
            _ => return Err(format!("unexpected argument '{arg}'")),
        }
    }

    Ok(options)
}

fn run_game(options: CliOptions) -> Result<(), String> {
    let mut ui = UiState::new(options)?;
    let mut guard = TerminalGuard::new()?;

    loop {
        if let Some(opp_lead) = ui
            .game
            .maybe_play_opponent_lead()
            .map_err(|error| format!("opponent turn failed: {error:?}"))?
        {
            ui.push_log(format!("Opponent leads {}", format_card(opp_lead)));
            ui.cached_hint = None;
            ui.selected_index = 0;
            ui.status = String::from("Your response turn.");
        }

        let hand_len = ui.game.my_hand().len();
        if hand_len > 0 && ui.selected_index >= hand_len {
            ui.selected_index = hand_len - 1;
        }

        if ui.hint_enabled && ui.game.is_player_turn() && ui.cached_hint.is_none() {
            match ui.game.hint_best_move() {
                Ok(hint) => {
                    ui.cached_hint = Some(hint);
                }
                Err(error) => {
                    ui.status = format!("Hint failed: {error:?}");
                }
            }
        }

        guard
            .terminal_mut()
            .draw(|frame| render(frame, &mut ui))
            .map_err(|error| format!("draw failed: {error}"))?;

        if !event::poll(Duration::from_millis(120))
            .map_err(|error| format!("poll failed: {error}"))?
        {
            continue;
        }

        let event = event::read().map_err(|error| format!("input read failed: {error}"))?;
        let Event::Key(key_event) = event else {
            continue;
        };
        if key_event.kind != KeyEventKind::Press {
            continue;
        }

        if ui.game.is_game_over() {
            match key_event.code {
                KeyCode::Char('q') | KeyCode::Enter | KeyCode::Esc => break,
                _ => continue,
            }
        }

        match key_event.code {
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Char('h') => {
                ui.hint_enabled = !ui.hint_enabled;
                ui.cached_hint = None;
                ui.status = if ui.hint_enabled {
                    String::from("Hint enabled")
                } else {
                    String::from("Hint disabled")
                };
            }
            KeyCode::Left | KeyCode::Up => {
                ui.selected_index = ui.selected_index.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Down => {
                if ui.selected_index + 1 < ui.game.my_hand().len() {
                    ui.selected_index += 1;
                }
            }
            KeyCode::Char(digit) if digit.is_ascii_digit() => {
                let Some(value) = digit.to_digit(10) else {
                    continue;
                };
                if value == 0 {
                    continue;
                }
                let Ok(target) = usize::try_from(value - 1) else {
                    continue;
                };
                if target < ui.game.my_hand().len() {
                    ui.selected_index = target;
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if !ui.game.is_player_turn() {
                    ui.status = String::from("Wait for opponent action");
                    continue;
                }

                let Some(chosen_card) = ui.game.my_hand().get(ui.selected_index).copied() else {
                    ui.status = String::from("No selectable card");
                    continue;
                };

                match ui.game.play_player_card(chosen_card) {
                    Ok(outcome) => {
                        ui.last_trick = Some(CompletedTrickView {
                            my_card: chosen_card,
                            opp_card: outcome.opponent_card,
                            winner: outcome.winner,
                        });
                        ui.status = format!(
                            "Played {} | Opp {} | Winner {:?} | +{}",
                            format_card(chosen_card),
                            format_card(outcome.opponent_card),
                            outcome.winner,
                            outcome.trick_points
                        );
                        ui.push_log(ui.status.clone());
                        ui.cached_hint = None;
                        animate_turn_winner_flash(&mut guard, &mut ui)?;
                        ui.last_trick = None;
                    }
                    Err(PlayError::InvalidMove) => {
                        ui.status = String::from("Invalid card selection");
                    }
                    Err(error) => {
                        ui.status = format!("Turn failed: {error:?}");
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn render(frame: &mut Frame<'_>, ui: &mut UiState) {
    let root = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(theme::BACKGROUND)), root);

    if root.width < 82 || root.height < 36 {
        render_small_terminal(frame, root);
        return;
    }

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Min(14),
            Constraint::Length(7),
            Constraint::Length(3),
        ])
        .split(root);

    render_header(frame, vertical[0], ui);

    let info_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(36),
            Constraint::Percentage(31),
            Constraint::Percentage(33),
        ])
        .split(vertical[1]);
    render_score_panel(frame, info_layout[0], ui);
    render_round_panel(frame, info_layout[1], ui);
    render_trump_panel(frame, info_layout[2], ui);

    let play_area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(vertical[2]);

    render_hand_cards(frame, play_area[0], ui);
    let side_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(7)])
        .split(play_area[1]);
    render_table_cards(frame, side_area[0], ui);
    render_hint_table(frame, side_area[1], ui);

    render_activity(frame, vertical[3], ui);
    render_footer(frame, vertical[4], ui);

    if ui.game.is_game_over() {
        render_game_over(frame, root, ui);
    }
}

fn render_small_terminal(frame: &mut Frame<'_>, area: Rect) {
    let message = Paragraph::new(Text::from(vec![
        Line::from(Span::styled(
            "brAIscola",
            Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::from("This table needs a little more room."),
        Line::from(Span::styled("Resize to at least 82 × 36.", Style::default().fg(theme::MUTED))),
        Line::default(),
        Line::from("q / Esc  quit"),
    ]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme::SURFACE).fg(theme::TEXT))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::GOLD)),
    );
    frame.render_widget(message, centered_fixed_rect(48, 10, area));
}

fn render_header(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let (turn, turn_color) = if ui.game.is_game_over() {
        ("MATCH COMPLETE", theme::GOLD)
    } else if ui.game.is_player_turn() {
        ("YOUR MOVE", theme::PLAYER)
    } else {
        ("AI THINKING", theme::OPPONENT)
    };
    let header = Paragraph::new(Line::from(vec![
        Span::styled("  br", Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
        Span::styled("AI", Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)),
        Span::styled("scola  ", Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)),
        Span::styled("  ◆  ", Style::default().fg(theme::GOLD_SOFT)),
        Span::styled(turn, Style::default().fg(turn_color).add_modifier(Modifier::BOLD)),
    ]))
    .alignment(Alignment::Center)
    .style(Style::default().bg(theme::SURFACE_RAISED))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::GOLD)),
    );
    frame.render_widget(header, area);
}

fn render_score_panel(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let block = panel(" SCORE — FIRST TO 61 ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);
    let player_score = ui.game.score_me();
    let opponent_score = ui.game.score_opp();
    frame.render_widget(
        LineGauge::default()
            .ratio(f64::from(player_score) / 120.0)
            .label(Line::from(Span::styled(
                format!("YOU  {player_score:>3} "),
                Style::default().fg(theme::PLAYER).add_modifier(Modifier::BOLD),
            )))
            .filled_style(Style::default().fg(theme::PLAYER))
            .unfilled_style(Style::default().fg(theme::BORDER)),
        rows[0],
    );
    frame.render_widget(
        LineGauge::default()
            .ratio(f64::from(opponent_score) / 120.0)
            .label(Line::from(Span::styled(
                format!("AI   {opponent_score:>3} "),
                Style::default().fg(theme::OPPONENT).add_modifier(Modifier::BOLD),
            )))
            .filled_style(Style::default().fg(theme::OPPONENT))
            .unfilled_style(Style::default().fg(theme::BORDER)),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("Trick {:02}/20", (ui.game.completed_tricks() + 1).min(20)),
            Style::default().fg(theme::MUTED),
        )))
        .alignment(Alignment::Right),
        rows[2],
    );
}

fn render_round_panel(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let leader_color = if ui.game.leader() == Player::Me { theme::PLAYER } else { theme::OPPONENT };
    let content = Text::from(vec![
        Line::from(vec![
            Span::styled("Leader     ", Style::default().fg(theme::MUTED)),
            Span::styled(
                player_label(ui.game.leader()),
                Style::default().fg(leader_color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Talon      ", Style::default().fg(theme::MUTED)),
            Span::styled(ui.game.talon_len().to_string(), Style::default().fg(theme::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("AI hand    ", Style::default().fg(theme::MUTED)),
            Span::styled(
                ui.game.opponent_cards_remaining().to_string(),
                Style::default().fg(theme::TEXT),
            ),
        ]),
        Line::from(vec![
            Span::styled("Seed       ", Style::default().fg(theme::MUTED)),
            Span::styled(ui.seed.to_string(), Style::default().fg(theme::BORDER)),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(content).style(Style::default().bg(theme::SURFACE)).block(panel(" ROUND ")),
        area,
    );
}

fn render_trump_panel(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let trump = ui.game.briscola_card();
    let opponent_played =
        ui.game.current_opponent_lead().map_or_else(|| String::from("waiting"), format_card);
    let content = Text::from(vec![
        Line::from(vec![
            Span::styled("Briscola   ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format_card(trump),
                Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {:?}", ui.game.briscola_suit()),
                Style::default().fg(theme::MUTED),
            ),
        ]),
        Line::from(vec![
            Span::styled("Value      ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("{} points", trump.rank.points()),
                Style::default().fg(theme::TEXT),
            ),
        ]),
        Line::default(),
        Line::from(vec![
            Span::styled("AI played  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                opponent_played,
                Style::default().fg(theme::INFO).add_modifier(Modifier::BOLD),
            ),
        ]),
    ]);
    frame.render_widget(
        Paragraph::new(content)
            .style(Style::default().bg(theme::SURFACE))
            .block(panel(" BRISCOLA ")),
        area,
    );
}

fn render_activity(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let lines: Vec<ListItem<'_>> = ui
        .log
        .iter()
        .rev()
        .take(5)
        .enumerate()
        .map(|(index, line)| {
            let style = if index == 0 {
                Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::MUTED)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    if index == 0 { "● " } else { "· " },
                    Style::default().fg(theme::GOLD),
                ),
                Span::styled(line.as_str(), style),
            ]))
        })
        .collect();
    frame.render_widget(
        List::new(lines).block(panel(" ACTIVITY ")).style(Style::default().bg(theme::SURFACE)),
        area,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    let status_color = if ui.status.contains("failed") || ui.status.contains("Invalid") {
        theme::OPPONENT
    } else {
        theme::PLAYER
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("● ", Style::default().fg(status_color)),
            Span::styled(ui.status.as_str(), Style::default().fg(theme::TEXT)),
        ]))
        .block(panel(" STATUS "))
        .style(Style::default().bg(theme::SURFACE)),
        columns[0],
    );

    let key = |label: &'static str| {
        Span::styled(
            label,
            Style::default().fg(theme::BACKGROUND).bg(theme::GOLD).add_modifier(Modifier::BOLD),
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            key(" ←/→ "),
            Span::styled(" select  ", Style::default().fg(theme::MUTED)),
            key(" ENTER "),
            Span::styled(" play  ", Style::default().fg(theme::MUTED)),
            key(" H "),
            Span::styled(" hint  ", Style::default().fg(theme::MUTED)),
            key(" Q "),
            Span::styled(" quit", Style::default().fg(theme::MUTED)),
        ]))
        .alignment(Alignment::Center)
        .block(panel(" KEYS "))
        .style(Style::default().bg(theme::SURFACE)),
        columns[1],
    );
}

fn render_game_over(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let popup_area = centered_fixed_rect(54, 9, area);
    frame.render_widget(Clear, popup_area);
    let (winner_text, winner_style) =
        match winner_from_scores(ui.game.score_me(), ui.game.score_opp()) {
            Some(Player::Me) => {
                ("YOU WIN", Style::default().fg(theme::PLAYER).add_modifier(Modifier::BOLD))
            }
            Some(Player::Opponent) => {
                ("OPPONENT WINS", Style::default().fg(theme::OPPONENT).add_modifier(Modifier::BOLD))
            }
            None => ("DRAW", Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)),
        };

    let popup = Paragraph::new(Text::from(vec![
        Line::default(),
        Line::from(Span::styled(winner_text, winner_style)),
        Line::default(),
        Line::from(vec![
            Span::styled(
                format!("{:>3}", ui.game.score_me()),
                Style::default().fg(theme::PLAYER).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  —  ", Style::default().fg(theme::MUTED)),
            Span::styled(
                format!("{:<3}", ui.game.score_opp()),
                Style::default().fg(theme::OPPONENT).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::default(),
        Line::from(Span::styled(
            "Enter / q / Esc  to leave the table",
            Style::default().fg(theme::MUTED),
        )),
    ]))
    .style(Style::default().bg(theme::SURFACE_RAISED).fg(theme::TEXT))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(theme::GOLD))
            .title(Span::styled(
                " MATCH COMPLETE ",
                Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD),
            )),
    );
    frame.render_widget(popup, popup_area);
}

fn panel(title: &'static str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::BORDER))
        .style(Style::default().bg(theme::SURFACE).fg(theme::TEXT))
        .title(Span::styled(title, Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)))
}

fn render_hint_table(frame: &mut Frame<'_>, area: Rect, ui: &UiState) {
    let hint_rows: Vec<Row<'_>> = if ui.hint_enabled {
        if let Some(hint) = ui.cached_hint.as_ref() {
            hint.moves
                .iter()
                .map(|stats| {
                    let is_best = stats.card == hint.best_move;
                    Row::new(vec![
                        Cell::from(Line::from(vec![
                            Span::styled(
                                if is_best { "★ " } else { "  " },
                                Style::default().fg(theme::GOLD),
                            ),
                            Span::raw(format_card(stats.card)),
                        ])),
                        Cell::from(format!("{:.0}%", stats.p_win * 100.0)),
                        Cell::from(format!("{:+.1}", stats.expected_score_delta)),
                    ])
                    .style(if is_best {
                        Style::default()
                            .fg(theme::PLAYER)
                            .bg(theme::SURFACE_RAISED)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::MUTED)
                    })
                })
                .collect()
        } else {
            vec![
                Row::new(vec![Cell::from("· thinking"), Cell::from("—"), Cell::from("—")])
                    .style(Style::default().fg(theme::MUTED)),
            ]
        }
    } else {
        vec![
            Row::new(vec![Cell::from("press H"), Cell::from("—"), Cell::from("—")])
                .style(Style::default().fg(theme::MUTED)),
        ]
    };

    let hint_table =
        Table::new(hint_rows, [Constraint::Min(8), Constraint::Length(6), Constraint::Length(8)])
            .header(
                Row::new(vec!["Card", "Win", "Δ score"])
                    .style(Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)),
            )
            .block(panel(if ui.hint_enabled { " ADVISOR ● ON " } else { " ADVISOR ○ OFF " }))
            .column_spacing(1)
            .style(Style::default().bg(theme::SURFACE));
    frame.render_widget(hint_table, area);
}

fn render_table_cards(frame: &mut Frame<'_>, area: Rect, ui: &mut UiState) {
    let outer = panel(" TABLE CARDS ");
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if inner.width < 6 || inner.height < 5 {
        return;
    }

    let (opp_card, my_card, winner) = if let Some(last_trick) = ui.last_trick {
        (Some(last_trick.opp_card), Some(last_trick.my_card), Some(last_trick.winner))
    } else {
        (ui.game.current_opponent_lead(), None, None)
    };

    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    let opp_highlight = ui.winner_flash_on && winner == Some(Player::Opponent);
    let my_highlight = ui.winner_flash_on && winner == Some(Player::Me);

    render_table_slot(
        frame,
        columns[0],
        TableSlotView { title: "Opponent", card: opp_card, is_winner_highlighted: opp_highlight },
        ui,
    );
    render_table_slot(
        frame,
        columns[1],
        TableSlotView { title: "Me", card: my_card, is_winner_highlighted: my_highlight },
        ui,
    );
}

fn render_table_slot(frame: &mut Frame<'_>, area: Rect, slot: TableSlotView<'_>, ui: &mut UiState) {
    let border_style = if slot.is_winner_highlighted {
        Style::default().fg(theme::PLAYER).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::BORDER)
    };

    let mut lines = Vec::new();
    if let Some(card) = slot.card {
        lines.push(Line::from(Span::styled(
            format_card(card),
            Style::default().fg(theme::INFO).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled("Value ", Style::default().fg(theme::MUTED)),
            Span::styled(format!("{} pts", card.rank.points()), Style::default().fg(theme::TEXT)),
        ]));
        lines.push(Line::default());

        match ui.table_renderer.render_card(card) {
            Ok(card_lines) => lines.extend(card_lines),
            Err(error) => {
                if ui.art_error.is_none() {
                    ui.art_error = Some(error.clone());
                    ui.status = format!("Card rendering disabled: {error}");
                }
            }
        }
    } else {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("· waiting ·", Style::default().fg(theme::MUTED))));
    }

    let title = if slot.is_winner_highlighted {
        format!(" ★ {} WINS ", slot.title)
    } else {
        format!(" {} ", slot.title)
    };
    let panel = Paragraph::new(Text::from(lines))
        .style(Style::default().bg(theme::SURFACE))
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    title,
                    if slot.is_winner_highlighted {
                        Style::default().fg(theme::PLAYER).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::MUTED)
                    },
                ))
                .border_style(border_style),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}

fn animate_turn_winner_flash(guard: &mut TerminalGuard, ui: &mut UiState) -> Result<(), String> {
    const FLASHES: usize = 3;
    const INTERVAL_MS: u64 = 300;

    for step in 0..(FLASHES * 2) {
        ui.winner_flash_on = step % 2 == 0;
        guard
            .terminal_mut()
            .draw(|frame| render(frame, ui))
            .map_err(|error| format!("draw failed: {error}"))?;
        thread::sleep(Duration::from_millis(INTERVAL_MS));
    }

    ui.winner_flash_on = false;
    clear_pending_input_events()?;
    Ok(())
}

fn clear_pending_input_events() -> Result<(), String> {
    while event::poll(Duration::from_millis(0)).map_err(|error| format!("poll failed: {error}"))? {
        let _ = event::read().map_err(|error| format!("input read failed: {error}"))?;
    }
    Ok(())
}

fn render_hand_cards(frame: &mut Frame<'_>, area: Rect, ui: &mut UiState) {
    let hand = ui.game.my_hand().to_vec();
    let card_constraints = vec![Constraint::Ratio(1, 3); HAND_SLOTS];
    let card_areas = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(card_constraints)
        .split(area);

    for (index, slot_area) in card_areas.iter().enumerate().take(HAND_SLOTS) {
        let Some(card) = hand.get(index).copied() else {
            let empty_card = Paragraph::new(Text::from(vec![
                Line::default(),
                Line::from(Span::styled("· empty slot ·", Style::default().fg(theme::MUTED))),
            ]))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .title(Span::styled(
                        format!(" {} ", index + 1),
                        Style::default().fg(theme::MUTED),
                    ))
                    .border_style(Style::default().fg(theme::BORDER)),
            )
            .style(Style::default().bg(theme::SURFACE))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
            frame.render_widget(empty_card, *slot_area);
            continue;
        };

        let is_selected = index == ui.selected_index;
        let is_hint =
            ui.cached_hint.as_ref().is_some_and(|hint| hint.best_move == card && ui.hint_enabled);
        let is_briscola = card.suit == ui.game.briscola_suit();
        let name_style = if is_briscola {
            Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::INFO).add_modifier(Modifier::BOLD)
        };
        let italian_name_style = if is_briscola {
            Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::TEXT)
        };

        let mut lines = vec![
            Line::from(Span::styled(card_name_english(card), name_style)),
            Line::from(Span::styled(card_name_italian(card), italian_name_style)),
            Line::from(vec![
                Span::styled("Value ", Style::default().fg(theme::MUTED)),
                Span::styled(
                    format!("{} points", card.rank.points()),
                    Style::default().fg(theme::TEXT),
                ),
                if is_briscola {
                    Span::styled("  ◆ BRISCOLA", Style::default().fg(theme::GOLD))
                } else {
                    Span::raw("")
                },
            ]),
            Line::default(),
        ];

        match ui.renderer.render_card(card) {
            Ok(card_art_lines) => {
                lines.extend(card_art_lines);
            }
            Err(error) => {
                if ui.art_error.is_none() {
                    ui.art_error = Some(error.clone());
                    ui.status = format!("Card rendering disabled: {error}");
                }
                lines.push(Line::from(Span::styled(
                    format_card(card),
                    Style::default().fg(theme::INFO),
                )));
            }
        }

        let border_style = if is_selected {
            Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)
        } else if is_hint {
            Style::default().fg(theme::PLAYER).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::BORDER)
        };
        let title = if is_selected {
            format!(" ▶ {}  SELECTED ", index + 1)
        } else if is_hint {
            format!(" ★ {}  ADVISOR PICK ", index + 1)
        } else {
            format!(" {} ", index + 1)
        };
        let card_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .style(Style::default().bg(if is_selected {
                theme::SURFACE_RAISED
            } else {
                theme::SURFACE
            }))
            .title(Span::styled(
                title,
                if is_selected {
                    Style::default().fg(theme::GOLD).add_modifier(Modifier::BOLD)
                } else if is_hint {
                    Style::default().fg(theme::PLAYER).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::MUTED)
                },
            ));

        let card_widget = Paragraph::new(Text::from(lines))
            .block(card_block)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false });
        frame.render_widget(card_widget, *slot_area);
    }
}

fn centered_fixed_rect(width: u16, height: u16, area: Rect) -> Rect {
    let popup_width = width.min(area.width);
    let popup_height = height.min(area.height);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(popup_height) / 2),
            Constraint::Length(popup_height),
            Constraint::Min(0),
        ])
        .split(area);

    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(area.width.saturating_sub(popup_width) / 2),
            Constraint::Length(popup_width),
            Constraint::Min(0),
        ])
        .split(vertical[1]);

    horizontal[1]
}

fn player_label(player: Player) -> &'static str {
    match player {
        Player::Me => "Me",
        Player::Opponent => "Opponent",
    }
}

fn main() {
    let options = match parse_cli_options() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };

    if let Err(error) = run_game(options) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;

    use super::*;

    fn render_at_size(width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        let mut ui = UiState::new(CliOptions { seed: 42, hint_samples: 1, opponent_samples: 1 })
            .expect("test game");

        terminal.draw(|frame| render(frame, &mut ui)).expect("render");
    }

    #[test]
    fn full_table_layout_renders() {
        render_at_size(120, 40);
    }

    #[test]
    fn small_terminal_fallback_renders() {
        render_at_size(70, 24);
    }
}
