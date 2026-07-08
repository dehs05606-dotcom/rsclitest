use crate::{config::Config, model_catalog::EffortMode, tools::project::ProjectScanner};
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Sparkline, Wrap},
    Frame, Terminal,
};
use std::{
    io,
    time::{Duration, Instant},
};

const CYAN: Color = Color::Rgb(64, 235, 255);
const CYAN_DARK: Color = Color::Rgb(23, 97, 126);
const GREEN: Color = Color::Rgb(91, 255, 143);
const PURPLE: Color = Color::Rgb(187, 134, 252);
const MAGENTA: Color = Color::Rgb(255, 92, 210);
const ORANGE: Color = Color::Rgb(255, 190, 90);
const RED: Color = Color::Rgb(255, 94, 120);
const BLUE: Color = Color::Rgb(94, 166, 255);
const DIM: Color = Color::Rgb(88, 108, 128);
const PANEL_BG: Color = Color::Rgb(9, 16, 24);
const PANEL_BG_2: Color = Color::Rgb(11, 24, 35);
const DEEP_BG: Color = Color::Rgb(3, 7, 12);

pub fn run(cfg: Config) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = NeonTui::new(cfg).run(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

struct NeonTui {
    cfg: Config,
    tree_lines: Vec<String>,
    history: Vec<HistoryLine>,
    logs: Vec<String>,
    composer: PromptComposer,
    tick: u64,
    started: Instant,
    overlay: Overlay,
    history_scroll: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Palette,
    Help,
}

#[derive(Clone)]
struct HistoryLine {
    time: String,
    tag: String,
    color: Color,
    text: String,
}

#[derive(Debug, Clone)]
struct PromptComposer {
    input: String,
    selected_suggestion: usize,
    command_history: Vec<String>,
    history_cursor: Option<usize>,
    chips: Vec<ContextChip>,
    selected_chip: usize,
}

#[derive(Debug, Clone)]
struct ContextChip {
    key: &'static str,
    label: &'static str,
    hint: &'static str,
    color: Color,
    active: bool,
}

#[derive(Debug, Clone)]
struct Suggestion {
    label: String,
    detail: String,
    insert: String,
    color: Color,
}

impl NeonTui {
    fn new(cfg: Config) -> Self {
        let scanner = ProjectScanner::new(cfg.project_root.clone());
        let tree = scanner.tree(650).unwrap_or_else(|e| format!("scan error: {e:#}"));
        let mut history = Vec::new();
        history.push(HistoryLine::system("AIA neon terminal initialized"));
        history.push(HistoryLine::agent(
            "PLANNER",
            BLUE,
            "Project graph indexed. Prompt box is a command palette + context router. Press Ctrl+P for palette, Tab to accept suggestions.",
        ));
        history.push(HistoryLine::agent(
            "VEGA-X",
            PURPLE,
            "Multi-agent orchestration online: planner, coder, reviewer, tester, security auditor.",
        ));
        history.push(HistoryLine::agent(
            "SENTINEL",
            RED,
            "Secret redaction guard enabled. API keys stay in environment variables only; never paste secrets into prompts.",
        ));

        Self {
            cfg,
            tree_lines: tree.lines().map(ToString::to_string).collect(),
            history,
            logs: vec![
                "Tool sandbox ready".to_string(),
                "Diff preview engine armed".to_string(),
                "Undo checkpoint path: .aia/undo".to_string(),
                "Prompt chips: @files @git @tests @security @docs @shell".to_string(),
                "Fast mode: /effort fast".to_string(),
            ],
            composer: PromptComposer::new(),
            tick: 0,
            started: Instant::now(),
            overlay: Overlay::None,
            history_scroll: 0,
        }
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            self.tick = self.tick.wrapping_add(1);

            if event::poll(Duration::from_millis(75))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            match key.code {
                                KeyCode::Char('c') => break,
                                KeyCode::Char('p') => self.toggle_palette(),
                                KeyCode::Char('l') => self.clear_history(),
                                KeyCode::Char('f') => self.composer.toggle_chip_by_key("@files"),
                                KeyCode::Char('g') => self.composer.toggle_chip_by_key("@git"),
                                KeyCode::Char('t') => self.composer.toggle_chip_by_key("@tests"),
                                KeyCode::Char('s') => self.composer.toggle_chip_by_key("@security"),
                                _ => {}
                            }
                            continue;
                        }

                        match self.overlay {
                            Overlay::Palette => {
                                if self.handle_palette_key(key.code) {
                                    break;
                                }
                            }
                            Overlay::Help => self.handle_help_key(key.code),
                            Overlay::None => {
                                if self.handle_main_key(key.code) {
                                    break;
                                }
                            }
                        }
                    }
                    Event::Resize(_, _) => {}
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn handle_main_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Esc => return true,
            KeyCode::Char('q') if self.composer.input.is_empty() => return true,
            KeyCode::Char(' ') => {
                if self.composer.input.is_empty() {
                    self.composer.toggle_selected_chip();
                } else {
                    self.composer.push_char(' ');
                }
            },
            KeyCode::Backspace => self.composer.backspace(),
            KeyCode::Enter => self.submit_input(),
            KeyCode::Tab => self.accept_suggestion(),
            KeyCode::Up => self.composer.previous_suggestion_or_history(),
            KeyCode::Down => self.composer.next_suggestion_or_history(),
            KeyCode::Left => self.composer.previous_chip(),
            KeyCode::Right => self.composer.next_chip(),
            KeyCode::Home => self.history_scroll = self.history.len(),
            KeyCode::End => self.history_scroll = 0,
            KeyCode::PageUp => self.history_scroll = self.history_scroll.saturating_add(6),
            KeyCode::PageDown => self.history_scroll = self.history_scroll.saturating_sub(6),
            KeyCode::F(1) => self.overlay = Overlay::Help,
            KeyCode::F(2) => self.toggle_palette(),
            KeyCode::F(3) => self.cycle_effort_fast(),
            KeyCode::F(4) => self.cycle_effort_max(),
            KeyCode::Char(c) => self.composer.push_char(c),
            _ => {}
        }
        false
    }

    fn handle_palette_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Char('q') => self.overlay = Overlay::None,
            KeyCode::Up => self.composer.previous_suggestion(),
            KeyCode::Down => self.composer.next_suggestion(),
            KeyCode::Tab | KeyCode::Enter => {
                self.accept_suggestion();
                self.overlay = Overlay::None;
            },
            KeyCode::Char(c) => self.composer.push_char(c),
            KeyCode::Backspace => self.composer.backspace(),
            _ => {}
        }
        false
    }

    fn handle_help_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => self.overlay = Overlay::None,
            _ => {}
        }
    }

    fn toggle_palette(&mut self) {
        self.overlay = if self.overlay == Overlay::Palette {
            Overlay::None
        } else {
            Overlay::Palette
        };
    }

    fn clear_history(&mut self) {
        self.history.clear();
        self.history_scroll = 0;
        self.history.push(HistoryLine::system("Terminal history cleared"));
    }

    fn submit_input(&mut self) {
        let input = self.composer.input.trim().to_string();
        self.composer.clear_input();
        if input.is_empty() {
            self.composer.toggle_selected_chip();
            return;
        }
        self.composer.remember(&input);
        let chips = self.composer.active_chip_keys().join(" ");
        self.history.push(HistoryLine::input(&format!("{} {}", chips, input).trim().to_string()));
        if input.starts_with('/') {
            self.handle_command(&input);
        } else {
            self.history.push(HistoryLine::agent(
                "AIA",
                CYAN,
                "Prompt accepted by the TUI composer. Use `aia chat` for current live LLM streaming; this deck is ready for the next bridge step.",
            ));
            self.logs.push(format!(
                "queued prompt with chips [{}]: {}",
                chips,
                truncate(&input, 72)
            ));
        }
        self.prune_buffers();
    }

    fn handle_command(&mut self, input: &str) {
        let mut parts = input.split_whitespace();
        let cmd = parts.next().unwrap_or_default();
        match cmd {
            "/help" => self.overlay = Overlay::Help,
            "/palette" | "/" => self.overlay = Overlay::Palette,
            "/status" => self.history.push(HistoryLine::system(&self.cfg.runtime_summary())),
            "/clear" => self.clear_history(),
            "/model" => {
                if let Some(model) = parts.next() {
                    self.cfg.model = model.to_string();
                    self.history
                        .push(HistoryLine::system(&format!("Model switched to {}", self.cfg.model)));
                } else {
                    self.history.push(HistoryLine::system(&format!(
                        "Current model: {}. Use /model <provider-model-name>",
                        self.cfg.model
                    )));
                }
            }
            "/provider" => {
                if let Some(provider) = parts.next() {
                    self.cfg.provider = provider.to_string();
                    self.history
                        .push(HistoryLine::system(&format!("Provider switched to {}", self.cfg.provider)));
                } else {
                    self.history.push(HistoryLine::system(&format!(
                        "Current provider: {}. Use /provider opencode|nvidia|ollama|openai|openrouter",
                        self.cfg.provider
                    )));
                }
            }
            "/effort" | "/mode" => {
                if let Some(mode) = parts.next() {
                    match mode.parse::<EffortMode>() {
                        Ok(mode) => self.apply_effort(mode),
                        Err(err) => self.history.push(HistoryLine::agent("ERROR", RED, &err)),
                    }
                } else {
                    self.history.push(HistoryLine::system(&format!(
                        "Current effort: {}. Use /effort fast|balanced|deep|max",
                        self.cfg.effort
                    )));
                }
            }
            "/chips" => {
                self.history.push(HistoryLine::system(&format!(
                    "Active chips: {}",
                    self.composer.active_chip_keys().join(" ")
                )));
            }
            _ => self
                .history
                .push(HistoryLine::agent("ERROR", RED, "Unknown command. Type /help or Ctrl+P.")),
        }
    }

    fn apply_effort(&mut self, mode: EffortMode) {
        self.cfg.apply_effort_with_recommended_model(mode);
        let profile = self.cfg.effort_profile();
        self.history.push(HistoryLine::agent(
            "EFFORT",
            GREEN,
            &format!(
                "{} enabled: model={}, scan_files={}, tool_iters={}, output_tokens={:?}",
                profile.label,
                self.cfg.model,
                self.cfg.context_scan_files,
                self.cfg.max_tool_iterations,
                self.cfg.effective_output_tokens()
            ),
        ));
        self.logs.push(format!("effort changed -> {} model={}", self.cfg.effort, self.cfg.model));
    }

    fn cycle_effort_fast(&mut self) {
        self.apply_effort(EffortMode::Fast);
    }

    fn cycle_effort_max(&mut self) {
        self.apply_effort(EffortMode::Max);
    }

    fn accept_suggestion(&mut self) {
        if let Some(suggestion) = self.composer.selected_suggestion() {
            self.composer.input = suggestion.insert;
            self.composer.selected_suggestion = 0;
        }
    }

    fn prune_buffers(&mut self) {
        if self.history.len() > 140 {
            let drain = self.history.len() - 140;
            self.history.drain(0..drain);
        }
        if self.logs.len() > 90 {
            let drain = self.logs.len() - 90;
            self.logs.drain(0..drain);
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Block::default().style(Style::default().bg(DEEP_BG)), area);

        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(18),
                Constraint::Length(10),
                Constraint::Length(1),
            ])
            .split(area);

        self.draw_header(frame, vertical[0]);
        self.draw_body(frame, vertical[1]);
        self.draw_prompt_box(frame, vertical[2]);
        self.draw_footer(frame, vertical[3]);

        match self.overlay {
            Overlay::Palette => self.draw_palette_overlay(frame, area),
            Overlay::Help => self.draw_help_overlay(frame, area),
            Overlay::None => {}
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let elapsed = self.started.elapsed().as_secs();
        let context = self
            .cfg
            .context_window_tokens()
            .map(format_tokens)
            .unwrap_or_else(|| "?".to_string());
        let title = Line::from(vec![
            Span::styled("  AI TERMINAL AGENT  ", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
            Span::styled("| ", Style::default().fg(DIM)),
            Span::styled(format!("Session #{:04} ", 1435 + (self.tick % 1000)), Style::default().fg(ORANGE)),
            Span::styled("| ", Style::default().fg(DIM)),
            Span::styled("v0.3-neon ", Style::default().fg(GREEN)),
            Span::styled("| ", Style::default().fg(DIM)),
            Span::styled(format!("{}:{} ", self.cfg.provider, self.cfg.model), Style::default().fg(PURPLE)),
            Span::styled("| ", Style::default().fg(DIM)),
            Span::styled(format!("effort={} ctx={} uptime={}s", self.cfg.effort, context, elapsed), Style::default().fg(BLUE)),
        ]);
        let header = Paragraph::new(title)
            .block(neon_block(" AIA CONTROL DECK ", CYAN))
            .alignment(Alignment::Center);
        frame.render_widget(header, area);
    }

    fn draw_body(&self, frame: &mut Frame, area: Rect) {
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(56),
                Constraint::Percentage(21),
                Constraint::Percentage(23),
            ])
            .split(area);

        let left_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(61), Constraint::Percentage(39)])
            .split(columns[0]);
        self.draw_terminal_history(frame, left_rows[0]);
        self.draw_diff_and_graph(frame, left_rows[1]);

        let middle_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
            .split(columns[1]);
        self.draw_files(frame, middle_rows[0]);
        self.draw_tool_logs(frame, middle_rows[1]);

        let right_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(23),
                Constraint::Percentage(34),
                Constraint::Percentage(25),
                Constraint::Percentage(18),
            ])
            .split(columns[2]);
        self.draw_agents(frame, right_rows[0]);
        self.draw_context_activity(frame, right_rows[1]);
        self.draw_tasks(frame, right_rows[2]);
        self.draw_token_router(frame, right_rows[3]);
    }

    fn draw_terminal_history(&self, frame: &mut Frame, area: Rect) {
        let max = area.height.saturating_sub(2) as usize;
        let total_lines: Vec<Line> = self
            .history
            .iter()
            .flat_map(|item| item.to_lines(area.width.saturating_sub(4) as usize))
            .collect();
        let visible_end = total_lines.len().saturating_sub(self.history_scroll);
        let visible_start = visible_end.saturating_sub(max);
        let lines = total_lines[visible_start..visible_end].to_vec();
        let title = if self.history_scroll == 0 {
            " Terminal History  Chat & Output ".to_string()
        } else {
            format!(" Terminal History  scroll +{} ", self.history_scroll)
        };
        let panel = Paragraph::new(lines)
            .block(neon_block_owned(title, GREEN))
            .wrap(Wrap { trim: false });
        frame.render_widget(panel, area);
    }

    fn draw_diff_and_graph(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
            .split(area);

        let diff = vec![
            Line::from(vec![Span::styled(" Code patch preview ", Style::default().fg(CYAN).add_modifier(Modifier::BOLD))]),
            Line::from(vec![Span::styled(" ┌ before ───────────┐  ┌ after ────────────┐", Style::default().fg(DIM))]),
            Line::from(vec![Span::styled("- raw scan every turn", Style::default().fg(RED)), Span::styled(" + cached project graph", Style::default().fg(GREEN))]),
            Line::from(vec![Span::styled("- prompt text only   ", Style::default().fg(RED)), Span::styled(" + chips + palette   ", Style::default().fg(GREEN))]),
            Line::from(vec![Span::styled("+ diff approval panel", Style::default().fg(GREEN))]),
            Line::from(vec![Span::styled("+ undo checkpoint    ", Style::default().fg(ORANGE))]),
        ];
        let diff_panel = Paragraph::new(diff)
            .block(neon_block(" Diff Viewer ", PURPLE))
            .wrap(Wrap { trim: false });
        frame.render_widget(diff_panel, chunks[0]);

        let graph_lines = vec![
            Line::from(vec![Span::styled("        planner", Style::default().fg(BLUE))]),
            Line::from(vec![Span::styled("          │", Style::default().fg(DIM))]),
            Line::from(vec![Span::styled(" search ─ coder ─ tests", Style::default().fg(GREEN))]),
            Line::from(vec![Span::styled("          │", Style::default().fg(DIM))]),
            Line::from(vec![Span::styled("       reviewer", Style::default().fg(PURPLE))]),
            Line::from(vec![Span::styled("          │", Style::default().fg(DIM))]),
            Line::from(vec![Span::styled("      sentinel", Style::default().fg(RED))]),
        ];
        let graph = Paragraph::new(graph_lines)
            .block(neon_block(" Agent Graph ", BLUE))
            .alignment(Alignment::Center);
        frame.render_widget(graph, chunks[1]);
    }

    fn draw_files(&self, frame: &mut Frame, area: Rect) {
        let max = area.height.saturating_sub(3) as usize;
        let items: Vec<ListItem> = self
            .tree_lines
            .iter()
            .take(max)
            .map(|line| {
                let color = if line.contains("src/") || line.contains(".rs") {
                    CYAN
                } else if line.contains("Cargo") || line.contains("README") || line.contains("SYSTEM") {
                    ORANGE
                } else if line.contains("docs") || line.contains("prompt") {
                    PURPLE
                } else {
                    DIM
                };
                ListItem::new(Line::from(vec![Span::styled(
                    truncate(line, area.width as usize),
                    Style::default().fg(color),
                )]))
            })
            .collect();
        let title = format!(" FILES  indexed={} ", self.tree_lines.len());
        let list = List::new(items).block(neon_block_owned(title, BLUE));
        frame.render_widget(list, area);
    }

    fn draw_tool_logs(&self, frame: &mut Frame, area: Rect) {
        let max = area.height.saturating_sub(2) as usize;
        let start = self.logs.len().saturating_sub(max);
        let items: Vec<ListItem> = self.logs[start..]
            .iter()
            .map(|line| {
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(GREEN)),
                    Span::styled(truncate(line, area.width as usize), Style::default().fg(Color::White)),
                ]))
            })
            .collect();
        frame.render_widget(List::new(items).block(neon_block(" Tool Logs ", GREEN)), area);
    }

    fn draw_agents(&self, frame: &mut Frame, area: Rect) {
        let pulse = if self.tick % 20 < 10 { "●" } else { "◉" };
        let lines = vec![
            Line::from(vec![Span::styled(format!("{pulse} ECHO-1 "), Style::default().fg(GREEN)), Span::raw("Idle")]),
            Line::from(vec![Span::styled("● NEXUS-2 ", Style::default().fg(CYAN)), Span::raw("Optimizing")]),
            Line::from(vec![Span::styled("✣ VEGA-X ", Style::default().fg(PURPLE)), Span::raw("Orchestrator")]),
            Line::from(vec![Span::styled("◆ SENTINEL ", Style::default().fg(RED)), Span::raw("Security")]),
        ];
        frame.render_widget(Paragraph::new(lines).block(neon_block(" AGENTS ", PURPLE)), area);
    }

    fn draw_context_activity(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Length(3), Constraint::Min(3)])
            .split(area);

        let heat = self.heatmap_lines();
        frame.render_widget(
            Paragraph::new(heat)
                .block(neon_block(" Enhanced Context  Neural Activity ", CYAN))
                .alignment(Alignment::Center),
            chunks[0],
        );

        let cpu_data = dynamic_data(self.tick, 26, 41);
        frame.render_widget(
            Sparkline::default()
                .block(Block::default().title(" CPU/RAM/GPU ").borders(Borders::LEFT | Borders::RIGHT))
                .data(&cpu_data)
                .style(Style::default().fg(GREEN)),
            chunks[1],
        );

        let context_window = self
            .cfg
            .context_window_tokens()
            .map(format_tokens)
            .unwrap_or_else(|| "unknown".to_string());
        let output_budget = self
            .cfg
            .effective_output_tokens()
            .map(format_tokens)
            .unwrap_or_else(|| "unknown".to_string());
        let meta = vec![
            Line::from(vec![Span::styled("Context window: ", Style::default().fg(DIM)), Span::styled(context_window, Style::default().fg(ORANGE))]),
            Line::from(vec![Span::styled("Scan files: ", Style::default().fg(DIM)), Span::styled(self.cfg.context_scan_files.to_string(), Style::default().fg(GREEN))]),
            Line::from(vec![Span::styled("Output budget: ", Style::default().fg(DIM)), Span::styled(output_budget, Style::default().fg(CYAN))]),
        ];
        frame.render_widget(Paragraph::new(meta).block(neon_block(" Runtime ", BLUE)), chunks[2]);
    }

    fn draw_tasks(&self, frame: &mut Frame, area: Rect) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(3),
            ])
            .split(area);
        let dynamic = ((self.tick % 100) as f64 / 100.0).max(0.08);
        let ratios = [0.28, 0.54, 0.72, 0.39, dynamic];
        let labels = ["Planner", "Coder", "Reviewer", "Tester", "Security"];
        let colors = [BLUE, GREEN, PURPLE, ORANGE, RED];
        for idx in 0..rows.len().min(labels.len()) {
            let gauge = Gauge::default()
                .block(Block::default().title(labels[idx]).borders(Borders::LEFT | Borders::RIGHT))
                .gauge_style(Style::default().fg(colors[idx]).bg(PANEL_BG).add_modifier(Modifier::BOLD))
                .ratio(ratios[idx]);
            frame.render_widget(gauge, rows[idx]);
        }
    }

    fn draw_token_router(&self, frame: &mut Frame, area: Rect) {
        let output = self.cfg.effective_output_tokens().unwrap_or_default();
        let context = self.cfg.context_window_tokens().unwrap_or(200_000);
        let ratio = (output as f64 / context.max(1) as f64).clamp(0.02, 1.0);
        let lines = vec![
            Line::from(vec![Span::styled("router: ", Style::default().fg(DIM)), Span::styled(&self.cfg.provider, Style::default().fg(CYAN))]),
            Line::from(vec![Span::styled("model: ", Style::default().fg(DIM)), Span::styled(truncate(&self.cfg.model, area.width as usize), Style::default().fg(PURPLE))]),
        ];
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(2), Constraint::Length(3)])
            .split(area);
        frame.render_widget(Paragraph::new(lines).block(neon_block(" Model Router ", ORANGE)), chunks[0]);
        frame.render_widget(
            Gauge::default()
                .block(Block::default().title(" output/context ").borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM))
                .gauge_style(Style::default().fg(CYAN).bg(PANEL_BG_2))
                .ratio(ratio),
            chunks[1],
        );
    }

    fn draw_prompt_box(&self, frame: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
            .split(area);

        let left_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(2)])
            .split(chunks[0]);

        let cursor = if self.tick % 12 < 6 { "█" } else { " " };
        let prompt = Paragraph::new(vec![Line::from(vec![
            Span::styled("[INPUT] ", Style::default().fg(GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("@terminal:~/project $ ", Style::default().fg(CYAN)),
            Span::styled(&self.composer.input, Style::default().fg(Color::White)),
            Span::styled(cursor, Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)),
        ])])
        .block(neon_block(" Prompt Box  command palette + context router ", CYAN))
        .wrap(Wrap { trim: false });
        frame.render_widget(prompt, left_rows[0]);

        self.draw_context_chips(frame, left_rows[1]);
        self.draw_prompt_status(frame, left_rows[2]);
        self.draw_autocomplete(frame, chunks[1]);
    }

    fn draw_context_chips(&self, frame: &mut Frame, area: Rect) {
        let mut spans = vec![Span::styled("Context chips: ", Style::default().fg(DIM))];
        for (idx, chip) in self.composer.chips.iter().enumerate() {
            let selected = idx == self.composer.selected_chip;
            let symbol = if chip.active { "●" } else { "○" };
            let mut style = Style::default().fg(if chip.active { chip.color } else { DIM });
            if selected {
                style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
            }
            spans.push(Span::styled(format!(" {symbol}{}:{} ", chip.key, chip.label), style));
        }
        spans.push(Span::styled("  ←/→ select  Space toggle  Ctrl+P palette", Style::default().fg(ORANGE)));
        frame.render_widget(
            Paragraph::new(vec![Line::from(spans)]).block(neon_block(" Context Router ", GREEN)),
            area,
        );
    }

    fn draw_prompt_status(&self, frame: &mut Frame, area: Rect) {
        let active: Vec<&str> = self.composer.chips.iter().filter(|c| c.active).map(|c| c.hint).collect();
        let input_chars = self.composer.input.chars().count();
        let lines = vec![
            Line::from(vec![
                Span::styled("Mode: ", Style::default().fg(DIM)),
                Span::styled(self.cfg.effort.to_string(), Style::default().fg(CYAN)),
                Span::styled("  Active context: ", Style::default().fg(DIM)),
                Span::styled(if active.is_empty() { "none".to_string() } else { active.join(" + ") }, Style::default().fg(GREEN)),
            ]),
            Line::from(vec![
                Span::styled("Chars: ", Style::default().fg(DIM)),
                Span::styled(input_chars.to_string(), Style::default().fg(ORANGE)),
                Span::styled("  Enter submit  Tab accept  F1 help  F2 palette  F3 fast  F4 max", Style::default().fg(DIM)),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines).block(neon_block(" Composer Status ", BLUE)), area);
    }

    fn draw_autocomplete(&self, frame: &mut Frame, area: Rect) {
        let suggestions = self.composer.suggestions();
        let max = area.height.saturating_sub(2) as usize;
        let items: Vec<ListItem> = suggestions
            .iter()
            .take(max)
            .enumerate()
            .map(|(idx, s)| {
                let selected = idx == self.composer.selected_suggestion;
                let marker = if selected { "▶" } else { " " };
                let style = if selected {
                    Style::default().fg(s.color).bg(CYAN_DARK).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(s.color)
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{marker} "), style),
                    Span::styled(truncate(&s.label, 28), style),
                    Span::styled(" — ", Style::default().fg(DIM)),
                    Span::styled(truncate(&s.detail, area.width.saturating_sub(34) as usize), Style::default().fg(DIM)),
                ]))
            })
            .collect();
        frame.render_widget(List::new(items).block(neon_block(" Smart Autocomplete ", ORANGE)), area);
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let text = Line::from(vec![
            Span::styled(" Ctrl+C quit ", Style::default().fg(RED)),
            Span::styled("| Ctrl+P palette ", Style::default().fg(CYAN)),
            Span::styled("| Ctrl+F/G/T/S toggle chips ", Style::default().fg(GREEN)),
            Span::styled("| PageUp/PageDown history ", Style::default().fg(ORANGE)),
            Span::styled("| no secrets in prompts ", Style::default().fg(PURPLE)),
        ]);
        frame.render_widget(Paragraph::new(text).style(Style::default().bg(DEEP_BG)), area);
    }

    fn draw_palette_overlay(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(74, 70, area);
        frame.render_widget(Clear, popup);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(8), Constraint::Length(5)])
            .split(popup);

        let search = Paragraph::new(Line::from(vec![
            Span::styled("Command palette > ", Style::default().fg(CYAN).add_modifier(Modifier::BOLD)),
            Span::styled(&self.composer.input, Style::default().fg(Color::White)),
        ]))
        .block(neon_block(" Palette Search ", CYAN));
        frame.render_widget(search, chunks[0]);

        self.draw_autocomplete(frame, chunks[1]);

        let help = Paragraph::new(vec![
            Line::from(vec![Span::styled("↑/↓ select   Tab/Enter accept   Esc close", Style::default().fg(ORANGE))]),
            Line::from(vec![Span::styled("Fast presets: /effort fast, /model stepfun-ai/step-3.7-flash, @files, @tests", Style::default().fg(DIM))]),
        ])
        .block(neon_block(" Palette Controls ", PURPLE));
        frame.render_widget(help, chunks[2]);
    }

    fn draw_help_overlay(&self, frame: &mut Frame, area: Rect) {
        let popup = centered_rect(78, 76, area);
        frame.render_widget(Clear, popup);
        let lines = vec![
            Line::from(vec![Span::styled("AIA Neon TUI Help", Style::default().fg(CYAN).add_modifier(Modifier::BOLD))]),
            Line::from(""),
            Line::from(vec![Span::styled("Prompt Box", Style::default().fg(GREEN).add_modifier(Modifier::BOLD))]),
            Line::from("  Type normally, Enter submits, Tab accepts selected autocomplete."),
            Line::from("  Left/Right chooses a context chip; Space toggles it when input is empty."),
            Line::from("  Ctrl+F/G/T/S toggles @files/@git/@tests/@security quickly."),
            Line::from(""),
            Line::from(vec![Span::styled("Slash Commands", Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))]),
            Line::from("  /effort fast | balanced | deep | max"),
            Line::from("  /model <name>     /provider <name>     /status     /clear"),
            Line::from(""),
            Line::from(vec![Span::styled("Panels", Style::default().fg(PURPLE).add_modifier(Modifier::BOLD))]),
            Line::from("  Terminal history, diff viewer, file tree, tool logs, agents, neural context, task gauges."),
            Line::from(""),
            Line::from(vec![Span::styled("Esc / q close help", Style::default().fg(RED))]),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .block(neon_block(" Help Overlay ", CYAN))
                .wrap(Wrap { trim: false }),
            popup,
        );
    }

    fn heatmap_lines(&self) -> Vec<Line<'static>> {
        let palette = [BLUE, CYAN, GREEN, ORANGE, RED, PURPLE, MAGENTA];
        (0..4)
            .map(|row| {
                let spans: Vec<Span> = (0..22)
                    .map(|col| {
                        let idx = ((self.tick as usize / 3) + row * 7 + col * 3) % palette.len();
                        let ch = match (row + col + self.tick as usize) % 6 {
                            0 => "▁",
                            1 => "▃",
                            2 => "▄",
                            3 => "▆",
                            4 => "█",
                            _ => "▇",
                        };
                        Span::styled(ch.to_string(), Style::default().fg(palette[idx]))
                    })
                    .collect();
                Line::from(spans)
            })
            .collect()
    }
}

impl PromptComposer {
    fn new() -> Self {
        Self {
            input: String::new(),
            selected_suggestion: 0,
            command_history: Vec::new(),
            history_cursor: None,
            selected_chip: 0,
            chips: vec![
                ContextChip::new("@files", "Files", "repo files", BLUE, true),
                ContextChip::new("@git", "Git", "git diff", GREEN, true),
                ContextChip::new("@tests", "Tests", "test plan", PURPLE, false),
                ContextChip::new("@security", "Security", "risk audit", RED, true),
                ContextChip::new("@docs", "Docs", "docs", ORANGE, false),
                ContextChip::new("@shell", "Shell", "shell", CYAN, false),
            ],
        }
    }

    fn push_char(&mut self, c: char) {
        self.input.push(c);
        self.selected_suggestion = 0;
        self.history_cursor = None;
    }

    fn backspace(&mut self) {
        self.input.pop();
        self.selected_suggestion = 0;
        self.history_cursor = None;
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.selected_suggestion = 0;
        self.history_cursor = None;
    }

    fn remember(&mut self, input: &str) {
        if self.command_history.last().map(String::as_str) != Some(input) {
            self.command_history.push(input.to_string());
        }
        if self.command_history.len() > 50 {
            self.command_history.remove(0);
        }
        self.history_cursor = None;
    }

    fn suggestions(&self) -> Vec<Suggestion> {
        let q = self.input.trim().to_ascii_lowercase();
        let mut out = Vec::new();

        let slash = [
            ("/effort fast", "flash latency + fast model", "/effort fast", GREEN),
            ("/effort balanced", "default coding behavior", "/effort balanced", BLUE),
            ("/effort deep", "more inspection + tests", "/effort deep", PURPLE),
            ("/effort max", "huge context profile", "/effort max", ORANGE),
            ("/status", "show runtime provider/model/tokens", "/status", CYAN),
            ("/clear", "clear terminal history", "/clear", RED),
            ("/provider opencode", "OpenCode Zen endpoint", "/provider opencode", GREEN),
            ("/provider nvidia", "NVIDIA NIM endpoint", "/provider nvidia", GREEN),
            ("/model deepseek-v4-flash-free", "200K fast OpenCode model", "/model deepseek-v4-flash-free", GREEN),
            ("/model mimo-v2.5-free", "1M OpenCode context", "/model mimo-v2.5-free", ORANGE),
            ("/model big-pickle", "200K OpenCode model", "/model big-pickle", PURPLE),
            ("/model z-ai/glm-5.2", "1M NVIDIA context", "/model z-ai/glm-5.2", ORANGE),
            ("/model z-ai/glm-5.1", "1M NVIDIA context", "/model z-ai/glm-5.1", ORANGE),
            ("/model deepseek-ai/deepseek-v4-pro", "1M NVIDIA deep coding", "/model deepseek-ai/deepseek-v4-pro", PURPLE),
            ("/model stepfun-ai/step-3.7-flash", "200K NVIDIA flash", "/model stepfun-ai/step-3.7-flash", GREEN),
        ];

        let chips = [
            ("@files summarize architecture", "attach repo tree + important files", "@files summarize architecture", BLUE),
            ("@git review current diff", "diff-aware code review", "@git review current diff", GREEN),
            ("@tests run safe verification", "suggest and run tests safely", "@tests run safe verification", PURPLE),
            ("@security audit commands and edits", "risk scan + secret guard", "@security audit commands and edits", RED),
            ("@docs update README and prompt docs", "documentation mode", "@docs update README and prompt docs", ORANGE),
            ("@shell propose safe commands", "approval-gated shell", "@shell propose safe commands", CYAN),
        ];

        let workflows = [
            ("fix build errors with tests", "scan errors, patch, run formatter/tests", "@files @tests fix build errors with tests", GREEN),
            ("review repo like senior engineer", "planner + reviewer + security pass", "@files @git @security review repo like senior engineer", PURPLE),
            ("refactor with diff preview", "minimal patch + undo checkpoint", "@files refactor with diff preview", CYAN),
            ("make response ultra fast", "switch to fast profile", "/effort fast", GREEN),
        ];

        for (label, detail, insert, color) in slash.into_iter().chain(chips).chain(workflows) {
            if q.is_empty()
                || label.to_ascii_lowercase().contains(&q)
                || detail.to_ascii_lowercase().contains(&q)
                || (q.starts_with('/') && label.starts_with('/'))
                || (q.starts_with('@') && label.starts_with('@'))
            {
                out.push(Suggestion::new(label, detail, insert, color));
            }
        }

        if out.is_empty() {
            out.push(Suggestion::new(
                "submit prompt",
                "Enter sends the current prompt with active chips",
                &self.input,
                CYAN,
            ));
        }
        out
    }

    fn selected_suggestion(&self) -> Option<Suggestion> {
        let suggestions = self.suggestions();
        suggestions.get(self.selected_suggestion.min(suggestions.len().saturating_sub(1))).cloned()
    }

    fn next_suggestion(&mut self) {
        let len = self.suggestions().len().max(1);
        self.selected_suggestion = (self.selected_suggestion + 1) % len;
    }

    fn previous_suggestion(&mut self) {
        let len = self.suggestions().len().max(1);
        self.selected_suggestion = if self.selected_suggestion == 0 {
            len - 1
        } else {
            self.selected_suggestion - 1
        };
    }

    fn next_suggestion_or_history(&mut self) {
        if self.input.is_empty() && !self.command_history.is_empty() {
            let next = self.history_cursor.unwrap_or(self.command_history.len().saturating_sub(1));
            let next = (next + 1).min(self.command_history.len().saturating_sub(1));
            self.history_cursor = Some(next);
            self.input = self.command_history[next].clone();
        } else {
            self.next_suggestion();
        }
    }

    fn previous_suggestion_or_history(&mut self) {
        if self.input.is_empty() && !self.command_history.is_empty() {
            let prev = self
                .history_cursor
                .unwrap_or(self.command_history.len())
                .saturating_sub(1);
            self.history_cursor = Some(prev);
            self.input = self.command_history[prev].clone();
        } else {
            self.previous_suggestion();
        }
    }

    fn previous_chip(&mut self) {
        if self.selected_chip == 0 {
            self.selected_chip = self.chips.len().saturating_sub(1);
        } else {
            self.selected_chip -= 1;
        }
    }

    fn next_chip(&mut self) {
        self.selected_chip = (self.selected_chip + 1) % self.chips.len().max(1);
    }

    fn toggle_selected_chip(&mut self) {
        if let Some(chip) = self.chips.get_mut(self.selected_chip) {
            chip.active = !chip.active;
        }
    }

    fn toggle_chip_by_key(&mut self, key: &str) {
        if let Some(chip) = self.chips.iter_mut().find(|chip| chip.key == key) {
            chip.active = !chip.active;
        }
    }

    fn active_chip_keys(&self) -> Vec<&'static str> {
        self.chips
            .iter()
            .filter(|chip| chip.active)
            .map(|chip| chip.key)
            .collect()
    }
}

impl ContextChip {
    fn new(key: &'static str, label: &'static str, hint: &'static str, color: Color, active: bool) -> Self {
        Self {
            key,
            label,
            hint,
            color,
            active,
        }
    }
}

impl Suggestion {
    fn new(label: &str, detail: &str, insert: &str, color: Color) -> Self {
        Self {
            label: label.to_string(),
            detail: detail.to_string(),
            insert: insert.to_string(),
            color,
        }
    }
}

impl HistoryLine {
    fn now() -> String {
        chrono::Local::now().format("%H:%M:%S").to_string()
    }

    fn system(text: &str) -> Self {
        Self {
            time: Self::now(),
            tag: "SYSTEM".to_string(),
            color: GREEN,
            text: text.to_string(),
        }
    }

    fn input(text: &str) -> Self {
        Self {
            time: Self::now(),
            tag: "INPUT".to_string(),
            color: ORANGE,
            text: text.to_string(),
        }
    }

    fn agent(tag: &str, color: Color, text: &str) -> Self {
        Self {
            time: Self::now(),
            tag: tag.to_string(),
            color,
            text: text.to_string(),
        }
    }

    fn to_lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        let prefix_len = 13 + self.tag.len();
        let wrap_width = width.saturating_sub(prefix_len).max(24);
        for (idx, chunk) in wrap_text(&self.text, wrap_width).into_iter().enumerate() {
            if idx == 0 {
                out.push(Line::from(vec![
                    Span::styled(format!("[{}] ", self.time), Style::default().fg(DIM)),
                    Span::styled(format!("[{}] ", self.tag), Style::default().fg(self.color).add_modifier(Modifier::BOLD)),
                    Span::styled(chunk, Style::default().fg(Color::White)),
                ]));
            } else {
                out.push(Line::from(vec![
                    Span::styled("             ", Style::default().fg(DIM)),
                    Span::styled(chunk, Style::default().fg(Color::White)),
                ]));
            }
        }
        out
    }
}

fn neon_block(title: &'static str, color: Color) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(PANEL_BG).fg(Color::White))
}

fn neon_block_owned(title: String, color: Color) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(PANEL_BG).fg(Color::White))
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn dynamic_data(tick: u64, len: usize, phase: u64) -> Vec<u64> {
    (0..len)
        .map(|i| {
            let base = ((tick + i as u64 * phase) % 100) as i64;
            let wave = ((base - 50).abs() as u64).min(80);
            15 + wave
        })
        .collect()
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.chars().count() <= width {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.chars().count() + word.chars().count() + 1 > width && !line.is_empty() {
            out.push(line);
            line = String::new();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn truncate(text: &str, width: usize) -> String {
    let limit = width.saturating_sub(2).max(8);
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(limit.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn format_tokens(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}
