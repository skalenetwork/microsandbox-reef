mod host;

pub use host::Alias;

use crate::rows::{AgentDetail, AgentRow, Event, RoleDetail, RoleRow};
use anyhow::{Context, Result};
use host::{Failure, Host};
use ratatui::crossterm::cursor::MoveTo;
use ratatui::crossterm::event::{self, Event as Input, KeyCode, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{Clear, ClearType};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use reef_core::{AgentName, RoleName, State, VmStatus};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(5);
const TAIL: &str = "12";
const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

#[derive(Clone, Copy)]
enum Tone {
    Plain,
    Muted,
    Ok,
    Warn,
    Bad,
}

impl Tone {
    fn state(state: State) -> Self {
        match state {
            State::Pending => Self::Warn,
            State::Running => Self::Ok,
            State::Stopped => Self::Muted,
            State::Failed => Self::Bad,
        }
    }

    fn vm(vm: Option<VmStatus>) -> Self {
        match vm {
            Some(VmStatus::Running) => Self::Ok,
            Some(VmStatus::Stopped) | None => Self::Muted,
        }
    }

    fn style(self, color: bool) -> Style {
        match (self, color) {
            (Self::Plain, _) => Style::new(),
            (Self::Muted, _) => Style::new().add_modifier(Modifier::DIM),
            (Self::Ok, true) => Style::new().fg(Color::Green),
            (Self::Warn, true) => Style::new().fg(Color::Yellow),
            (Self::Bad, true) => Style::new().fg(Color::Red),
            (Self::Ok | Self::Warn, false) => Style::new(),
            (Self::Bad, false) => Style::new().add_modifier(Modifier::BOLD),
        }
    }
}

pub fn hosts(aliases: Vec<Alias>, reef: String, state: PathBuf) -> Result<Vec<Host>> {
    if aliases.is_empty() {
        let exe = std::env::current_exe().context("cannot locate this binary")?;
        return Ok(vec![Host::Local { exe, state }]);
    }
    Ok(aliases
        .into_iter()
        .map(|alias| Host::Ssh {
            alias,
            reef: reef.clone(),
        })
        .collect())
}

pub fn run(hosts: Vec<Host>) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let hosts = hosts
        .into_iter()
        .enumerate()
        .map(|(index, host)| {
            let (wake, wakes) = mpsc::channel();
            poll(index, host.clone(), tx.clone(), wakes);
            HostState {
                host,
                agents: None,
                roles: None,
                wake,
            }
        })
        .collect();
    let mut app = App::new(hosts, tx);
    let mut terminal = ratatui::try_init()?;
    let outcome = app.drive(&mut terminal, &rx);
    ratatui::restore();
    outcome
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    Agents,
    Roles,
}

struct Spec {
    columns: &'static [&'static str],
    list: &'static [&'static str],
    empty: &'static str,
    other: &'static str,
}

impl View {
    fn spec(self) -> Spec {
        match self {
            Self::Agents => Spec {
                columns: &[
                    "host", "name", "role", "owner", "desired", "state", "vm", "sync", "ports",
                ],
                list: &["agent", "list"],
                empty: "no agents",
                other: "roles",
            },
            Self::Roles => Spec {
                columns: &["host", "name", "version", "image", "agents", "stale"],
                list: &["role", "list"],
                empty: "no roles",
                other: "agents",
            },
        }
    }
}

#[derive(Clone, Copy)]
struct Verb {
    key: char,
    label: &'static str,
    arg: &'static str,
    progress: &'static str,
    asks: bool,
}

#[rustfmt::skip] const START:  Verb = Verb { key: 's', label: "start",  arg: "start",  progress: "starting", asks: false };
#[rustfmt::skip] const STOP:   Verb = Verb { key: 'x', label: "stop",   arg: "stop",   progress: "stopping", asks: false };
#[rustfmt::skip] const UPDATE: Verb = Verb { key: 'u', label: "update", arg: "update", progress: "updating", asks: true };
#[rustfmt::skip] const REMOVE: Verb = Verb { key: 'd', label: "remove", arg: "rm",     progress: "removing", asks: true };

enum Msg {
    Agents(usize, Result<Vec<AgentRow>, Failure>),
    Roles(usize, Result<Vec<RoleRow>, Failure>),
    Detail(Row, Result<Detail, Failure>),
    Done(Row, Result<(), Failure>),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Row {
    Agent(usize, AgentName),
    Role(usize, RoleName),
}

impl Row {
    fn host(&self) -> usize {
        match self {
            Self::Agent(host, _) | Self::Role(host, _) => *host,
        }
    }

    fn name(&self) -> &str {
        match self {
            Self::Agent(_, name) => name.as_str(),
            Self::Role(_, name) => name.as_str(),
        }
    }

    fn noun(&self) -> &'static str {
        match self {
            Self::Agent(..) => "agent",
            Self::Role(..) => "role",
        }
    }

    fn is_agent(&self) -> bool {
        matches!(self, Self::Agent(..))
    }

    fn verbs(&self) -> &'static [Verb] {
        match self {
            Self::Agent(..) => &[START, STOP, UPDATE, REMOVE],
            Self::Role(..) => &[REMOVE],
        }
    }
}

enum Detail {
    Agent(Box<AgentDetail>, Result<Vec<Event>, Failure>),
    Role(Box<RoleDetail>),
}

struct HostState {
    host: Host,
    agents: Option<Result<Vec<AgentRow>, Failure>>,
    roles: Option<Result<Vec<RoleRow>, Failure>>,
    wake: Sender<View>,
}

enum Screen {
    Table,
    Detail {
        row: Row,
        detail: Option<Result<Detail, Failure>>,
        scroll: u16,
        polled: Instant,
    },
}

enum Item<'a> {
    Agent(usize, &'a AgentRow),
    Role(usize, &'a RoleRow),
    Host(usize, String, Tone),
}

impl Item<'_> {
    fn row(&self) -> Option<Row> {
        match self {
            Self::Agent(host, agent) => Some(Row::Agent(*host, agent.name.clone())),
            Self::Role(host, role) => Some(Row::Role(*host, role.name.clone())),
            Self::Host(..) => None,
        }
    }
}

struct App {
    hosts: Vec<HostState>,
    view: View,
    selected: usize,
    screen: Screen,
    busy: BTreeMap<Row, Verb>,
    confirm: Option<(Verb, Row)>,
    shell: Option<Row>,
    flash: Option<String>,
    quit: bool,
    color: bool,
    tx: Sender<Msg>,
}

impl App {
    fn new(hosts: Vec<HostState>, tx: Sender<Msg>) -> Self {
        Self {
            hosts,
            view: View::Agents,
            selected: 0,
            screen: Screen::Table,
            busy: BTreeMap::new(),
            confirm: None,
            shell: None,
            flash: None,
            quit: false,
            color: color(),
            tx,
        }
    }

    fn drive(&mut self, terminal: &mut DefaultTerminal, rx: &Receiver<Msg>) -> Result<()> {
        while !self.quit {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(50))?
                && let Input::Key(key) = event::read()?
                && key.is_press()
            {
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    self.quit = true;
                } else {
                    self.key(key.code);
                }
            }
            for msg in rx.try_iter() {
                self.apply(msg);
            }
            self.refresh();
            if let Some(row) = self.shell.take() {
                self.hand_over(terminal, &row)?;
            }
        }
        Ok(())
    }

    fn hand_over(&mut self, terminal: &mut DefaultTerminal, row: &Row) -> Result<()> {
        let mut shell = self.hosts[row.host()].host.shell(row.name());
        ratatui::restore();
        execute!(std::io::stdout(), Clear(ClearType::All), MoveTo(0, 0)).ok();
        let status = shell.status();
        *terminal = ratatui::try_init()?;
        terminal.clear()?;
        self.flash = status.err().map(|e| format!("{}: {e}", row.name()));
        Ok(())
    }

    fn items(&self) -> Vec<Item<'_>> {
        let empty = self.view.spec().empty;
        self.hosts
            .iter()
            .enumerate()
            .flat_map(|(index, host)| match self.view {
                View::Agents => listing(index, &host.agents, empty, Item::Agent),
                View::Roles => listing(index, &host.roles, empty, Item::Role),
            })
            .collect()
    }

    fn selected_row(&self) -> Option<Row> {
        self.items().get(self.selected)?.row()
    }

    fn position(&self, row: &Row) -> Option<usize> {
        self.items()
            .iter()
            .position(|item| item.row().as_ref() == Some(row))
    }

    fn target(&self) -> Option<Row> {
        match &self.screen {
            Screen::Detail { row, .. } => Some(row.clone()),
            Screen::Table => self.selected_row(),
        }
    }

    fn key(&mut self, code: KeyCode) {
        self.flash = None;
        if let Some((verb, row)) = self.confirm.take() {
            if code == KeyCode::Char('y') {
                self.act(verb, row);
            }
            return;
        }
        let table = matches!(self.screen, Screen::Table);
        match code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => self.screen = Screen::Table,
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Enter if table => self.open(),
            KeyCode::Tab if table => self.switch(),
            KeyCode::Char('t') => self.shell = self.target().filter(Row::is_agent),
            KeyCode::Char(key) => self.press(key),
            _ => {}
        }
    }

    fn step(&mut self, delta: i16) {
        if let Screen::Detail { scroll, .. } = &mut self.screen {
            *scroll = scroll.saturating_add_signed(delta);
            return;
        }
        self.selected = self.clamp(self.selected.saturating_add_signed(delta.into()));
    }

    fn clamp(&self, index: usize) -> usize {
        index.min(self.items().len().saturating_sub(1))
    }

    fn switch(&mut self) {
        self.view = match self.view {
            View::Agents => View::Roles,
            View::Roles => View::Agents,
        };
        self.selected = 0;
        for host in &self.hosts {
            host.wake.send(self.view).ok();
        }
    }

    fn reselect(&mut self, row: Option<Row>) {
        self.selected = row
            .and_then(|row| self.position(&row))
            .unwrap_or_else(|| self.clamp(self.selected));
    }

    fn press(&mut self, key: char) {
        let Some(row) = self.target() else {
            return;
        };
        let Some(verb) = row.verbs().iter().copied().find(|verb| verb.key == key) else {
            return;
        };
        match verb.asks {
            true => self.confirm = Some((verb, row)),
            false => self.act(verb, row),
        }
    }

    fn open(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        self.fetch(row.clone());
        self.screen = Screen::Detail {
            row,
            detail: None,
            scroll: 0,
            polled: Instant::now(),
        };
    }

    fn fetch(&self, row: Row) {
        let host = self.hosts[row.host()].host.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let detail = match &row {
                Row::Agent(_, name) => host.fetch(&["agent", "get", name.as_str()]).map(|it| {
                    let events = host.fetch(&["events", "--agent", name.as_str(), "--limit", TAIL]);
                    Detail::Agent(Box::new(it), events)
                }),
                Row::Role(_, name) => host
                    .fetch(&["role", "get", name.as_str()])
                    .map(|it| Detail::Role(Box::new(it))),
            };
            tx.send(Msg::Detail(row, detail)).ok();
        });
    }

    fn refresh(&mut self) {
        let row = match &mut self.screen {
            Screen::Detail { row, polled, .. } if polled.elapsed() >= POLL => {
                *polled = Instant::now();
                row.clone()
            }
            _ => return,
        };
        self.fetch(row);
    }

    fn act(&mut self, verb: Verb, row: Row) {
        if self.busy.contains_key(&row) {
            self.flash = Some(format!("{} is busy", row.name()));
            return;
        }
        self.busy.insert(row.clone(), verb);
        let host = self.hosts[row.host()].host.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcome = host.run(&[row.noun(), verb.arg, row.name()]);
            tx.send(Msg::Done(row, outcome)).ok();
        });
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Agents(index, agents) => {
                let anchor = self.selected_row();
                self.hosts[index].agents = Some(agents);
                self.reselect(anchor);
            }
            Msg::Roles(index, roles) => {
                let anchor = self.selected_row();
                self.hosts[index].roles = Some(roles);
                self.reselect(anchor);
            }
            Msg::Detail(target, fetched) => {
                if let Screen::Detail { row, detail, .. } = &mut self.screen
                    && *row == target
                {
                    *detail = Some(fetched);
                }
            }
            Msg::Done(row, outcome) => {
                self.busy.remove(&row);
                self.hosts[row.host()].wake.send(self.view).ok();
                if let Err(failure) = outcome {
                    self.flash = Some(format!("{}: {failure}", row.name()));
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let [body, footer] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)])
            .horizontal_margin(1)
            .areas(frame.area());
        match &self.screen {
            Screen::Detail {
                row,
                detail,
                scroll,
                ..
            } => {
                let [left, right] =
                    Layout::horizontal([Constraint::Min(0), Constraint::Percentage(42)])
                        .areas(body);
                let split = row.is_agent();
                let area = if split { left } else { body };
                frame.render_widget(self.detail(row, detail.as_ref()).scroll((*scroll, 0)), area);
                if split {
                    frame.render_widget(self.events(detail.as_ref()), right);
                }
            }
            Screen::Table => frame.render_widget(self.table(body), body),
        }
        frame.render_widget(self.footer(), footer);
    }

    fn style(&self, tone: Tone) -> Style {
        tone.style(self.color)
    }

    fn field(&self, label: &str, value: String, tone: Tone) -> Line<'static> {
        Line::from(vec![
            Span::styled(pad(label, 10), self.style(Tone::Muted)),
            Span::styled(value, self.style(tone)),
        ])
    }

    fn table(&self, area: Rect) -> Paragraph<'static> {
        let columns = self.view.spec().columns;
        let items = self.items();
        let first = usize::from(self.hosts.len() == 1);
        let cells: Vec<Vec<(String, Tone)>> = items.iter().map(|item| self.cells(item)).collect();
        let widths: Vec<usize> = (0..columns.len())
            .map(|column| {
                cells
                    .iter()
                    .filter_map(|row| row.get(column))
                    .map(|(text, _)| text.chars().count())
                    .chain([columns[column].len()])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let header = columns[first..]
            .iter()
            .zip(&widths[first..])
            .map(|(name, width)| Span::styled(pad(name, *width), self.style(Tone::Muted)))
            .collect::<Vec<_>>();
        let visible = usize::from(area.height.saturating_sub(1));
        let offset = self.selected.saturating_sub(visible.saturating_sub(1));
        let mut lines = vec![Line::from(header)];
        for (position, (item, cells)) in items.iter().zip(&cells).enumerate().skip(offset) {
            let spans = match item {
                Item::Host(index, status, tone) => [
                    Span::styled(
                        pad(self.hosts[*index].host.label(), widths[0]),
                        self.style(Tone::Muted),
                    ),
                    Span::styled(status.clone(), self.style(*tone)),
                ][first..]
                    .to_vec(),
                _ => cells[first..]
                    .iter()
                    .zip(&widths[first..])
                    .map(|((text, tone), width)| Span::styled(pad(text, *width), self.style(*tone)))
                    .collect(),
            };
            let mut line = Line::from(spans);
            if position == self.selected {
                let fill = usize::from(area.width).saturating_sub(line.width());
                line.push_span(" ".repeat(fill));
                line = line.style(SELECTED);
            }
            lines.push(line);
        }
        Paragraph::new(lines)
    }

    fn cells(&self, item: &Item) -> Vec<(String, Tone)> {
        let busy = item.row().and_then(|row| self.busy.get(&row)).copied();
        match item {
            Item::Agent(index, agent) => {
                let host = &self.hosts[*index];
                let (state, tone) = match busy {
                    Some(verb) => (verb.progress, Tone::Warn),
                    None => (agent.state.label(), Tone::state(agent.state)),
                };
                let role = if agent.role_current {
                    (agent.role.to_string(), Tone::Plain)
                } else {
                    (format!("{} stale", agent.role), Tone::Warn)
                };
                let ports: Vec<String> = agent
                    .ports
                    .iter()
                    .map(|(name, port)| format!("{name}={port}"))
                    .collect();
                vec![
                    (host.host.label().to_owned(), Tone::Muted),
                    (agent.name.to_string(), Tone::Plain),
                    role,
                    (agent.owner.clone(), Tone::Muted),
                    (agent.desired.label().to_owned(), Tone::Muted),
                    (state.to_owned(), tone),
                    (
                        agent.vm.map_or("-", VmStatus::label).to_owned(),
                        Tone::vm(agent.vm),
                    ),
                    if agent.synced {
                        ("yes".to_owned(), Tone::Muted)
                    } else {
                        ("drift".to_owned(), Tone::Warn)
                    },
                    (ports.join(" "), Tone::Plain),
                ]
            }
            Item::Role(index, role) => {
                let name = match busy {
                    Some(verb) => (format!("{} {}", role.name, verb.progress), Tone::Warn),
                    None => (role.name.to_string(), Tone::Plain),
                };
                vec![
                    (self.hosts[*index].host.label().to_owned(), Tone::Muted),
                    name,
                    (role.digest.short().to_owned(), Tone::Muted),
                    (role.image.to_string(), Tone::Plain),
                    (role.agents.to_string(), Tone::Plain),
                    (
                        role.stale.to_string(),
                        if role.stale > 0 {
                            Tone::Warn
                        } else {
                            Tone::Muted
                        },
                    ),
                ]
            }
            Item::Host(..) => Vec::new(),
        }
    }

    fn heading(&self, title: String) -> Vec<Line<'static>> {
        let rule = "─".repeat(title.chars().count());
        vec![
            Line::styled(title, self.style(Tone::Plain)),
            Line::styled(rule, self.style(Tone::Muted)),
        ]
    }

    fn detail(&self, row: &Row, detail: Option<&Result<Detail, Failure>>) -> Paragraph<'static> {
        let host = &self.hosts[row.host()].host;
        let mut lines = self.heading(format!("{} {}", row.noun(), row.name()));
        lines.extend(match detail {
            None => vec![self.field("state", "loading".to_owned(), Tone::Muted)],
            Some(Err(failure)) => vec![self.field("error", failure.to_string(), Tone::Bad)],
            Some(Ok(Detail::Role(role))) => role
                .rows()
                .into_iter()
                .map(|(label, value)| self.field(label, value, Tone::Plain))
                .collect(),
            Some(Ok(Detail::Agent(agent, _))) => {
                let mut fields: Vec<Line<'static>> = agent
                    .rows()
                    .into_iter()
                    .map(|(label, value)| {
                        let tone = match label {
                            "state" => Tone::state(agent.state),
                            "vm" => Tone::vm(agent.vm),
                            _ => Tone::Plain,
                        };
                        self.field(label, value, tone)
                    })
                    .collect();
                fields.extend(
                    agent
                        .ports
                        .values()
                        .filter_map(|port| host.forward(*port))
                        .map(|forward| self.field("forward", forward, Tone::Muted)),
                );
                fields
            }
        });
        Paragraph::new(lines).wrap(Wrap { trim: false })
    }

    fn events(&self, detail: Option<&Result<Detail, Failure>>) -> Paragraph<'static> {
        let now = now();
        let mut lines = self.heading("events".to_owned());
        lines.extend(match detail {
            Some(Ok(Detail::Agent(_, Err(failure)))) => {
                vec![Line::styled(failure.to_string(), self.style(Tone::Bad))]
            }
            Some(Ok(Detail::Agent(_, Ok(events)))) if events.is_empty() => {
                vec![Line::styled("none yet", self.style(Tone::Muted))]
            }
            Some(Ok(Detail::Agent(_, Ok(events)))) => events
                .iter()
                .rev()
                .map(|event| {
                    Line::from(vec![
                        Span::styled(pad(&event.kind, 8), self.style(Tone::Plain)),
                        Span::styled(
                            format!("{:>4}  ", ago(now, event.at)),
                            self.style(Tone::Muted),
                        ),
                        Span::styled(event.detail.clone(), self.style(Tone::Muted)),
                    ])
                })
                .collect(),
            _ => Vec::new(),
        });
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::LEFT)
                .border_style(self.style(Tone::Muted))
                .padding(Padding::left(2)),
        )
    }

    fn footer(&self) -> Line<'static> {
        if let Some((verb, row)) = &self.confirm {
            return Line::from(format!(
                "{} {} on {}? y/n",
                verb.arg,
                row.name(),
                self.hosts[row.host()].host.label()
            ));
        }
        if let Some(flash) = &self.flash {
            return Line::styled(flash.clone(), self.style(Tone::Bad));
        }
        Line::styled(self.keys(), self.style(Tone::Muted))
    }

    fn keys(&self) -> String {
        let row = self.target();
        let hints = row
            .iter()
            .flat_map(|row| row.verbs())
            .map(|verb| format!("{} {}  ", verb.key, verb.label))
            .collect::<String>();
        let terminal = match row.as_ref().is_some_and(Row::is_agent) {
            true => "t terminal  ",
            false => "",
        };
        match &self.screen {
            Screen::Table => format!(
                "j/k move  enter detail  {hints}{terminal}tab {}  q quit",
                self.view.spec().other
            ),
            Screen::Detail { .. } => format!("j/k scroll  esc back  {hints}{terminal}q quit"),
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs() as i64)
}

fn ago(now: i64, at: i64) -> String {
    let seconds = now.saturating_sub(at).max(0);
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

fn color() -> bool {
    std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty())
}

fn poll(index: usize, host: Host, tx: Sender<Msg>, wake: Receiver<View>) {
    thread::spawn(move || {
        let mut view = View::Agents;
        loop {
            let list = view.spec().list;
            let msg = match view {
                View::Agents => Msg::Agents(index, host.fetch(list)),
                View::Roles => Msg::Roles(index, host.fetch(list)),
            };
            if tx.send(msg).is_err() {
                return;
            }
            match wake.recv_timeout(POLL) {
                Ok(next) => view = wake.try_iter().last().unwrap_or(next),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

fn listing<'a, T>(
    index: usize,
    fetched: &'a Option<Result<Vec<T>, Failure>>,
    empty: &str,
    item: fn(usize, &'a T) -> Item<'a>,
) -> Vec<Item<'a>> {
    match fetched {
        None => vec![Item::Host(index, "connecting".to_owned(), Tone::Muted)],
        Some(Err(failure)) => vec![Item::Host(index, failure.to_string(), Tone::Bad)],
        Some(Ok(rows)) if rows.is_empty() => {
            vec![Item::Host(index, empty.to_owned(), Tone::Muted)]
        }
        Some(Ok(rows)) => rows.iter().map(|row| item(index, row)).collect(),
    }
}

fn pad(text: &str, width: usize) -> String {
    format!("{text:<width$}   ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::AgentResources;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use reef_core::Desired;

    fn agent(name: &str, state: State, synced: bool, ports: &[(&str, u16)]) -> AgentRow {
        AgentRow {
            name: name.parse().unwrap(),
            role: "echo".parse().unwrap(),
            role_digest: "0".repeat(64).parse().unwrap(),
            role_current: synced,
            image: "alpine".parse().unwrap(),
            owner: "ana".to_owned(),
            desired: Desired::Running,
            state,
            vm: (state == State::Running).then_some(VmStatus::Running),
            synced,
            ports: ports
                .iter()
                .map(|(name, port)| (name.parse().unwrap(), *port))
                .collect(),
        }
    }

    fn role(name: &str, agents: usize, stale: usize) -> RoleRow {
        RoleRow {
            name: name.parse().unwrap(),
            digest: "0".repeat(64).parse().unwrap(),
            image: "alpine".parse().unwrap(),
            agents,
            stale,
        }
    }

    fn state(host: Host, agents: Option<Result<Vec<AgentRow>, Failure>>) -> HostState {
        HostState {
            host,
            agents,
            roles: None,
            wake: mpsc::channel().0,
        }
    }

    fn app(hosts: Vec<HostState>) -> App {
        let mut app = App::new(hosts, mpsc::channel().0);
        app.color = true;
        app
    }

    fn ssh(alias: &str) -> Host {
        Host::Ssh {
            alias: alias.parse().unwrap(),
            reef: "reef".to_owned(),
        }
    }

    fn screen(app: &App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .chunks(usize::from(buffer.area.width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    #[test]
    fn table_shows_every_host_and_the_verbs() {
        let mut app = app(vec![
            state(
                ssh("prod-eu"),
                Some(Ok(vec![
                    agent("echo-1", State::Running, true, &[("ui", 19007)]),
                    agent("echo-2", State::Failed, false, &[]),
                ])),
            ),
            state(
                ssh("prod-us"),
                Some(Err(Failure::Unreachable("ssh: connect refused".to_owned()))),
            ),
        ]);
        app.busy
            .insert(Row::Agent(0, "echo-2".parse().unwrap()), UPDATE);
        assert_eq!(
            screen(&app, 110, 6),
            [
                " host      name     role         owner   desired   state      vm        sync    ports                         ",
                " prod-eu   echo-1   echo         ana     running   running    running   yes     ui=19007                      ",
                " prod-eu   echo-2   echo stale   ana     running   updating   -         drift                                 ",
                " prod-us   unreachable: ssh: connect refused                                                                  ",
                "                                                                                                              ",
                " j/k move  enter detail  s start  x stop  u update  d remove  t terminal  tab roles  q quit                   ",
            ]
        );
    }

    #[test]
    fn roles_view_counts_the_agents_on_each_role() {
        let mut app = app(vec![state(ssh("prod-eu"), None)]);
        app.hosts[0].roles = Some(Ok(vec![role("echo", 4, 1), role("builder", 2, 0)]));
        app.view = View::Roles;
        assert_eq!(
            screen(&app, 64, 5),
            [
                " name      version        image    agents   stale               ",
                " echo      000000000000   alpine   4        1                   ",
                " builder   000000000000   alpine   2        0                   ",
                "                                                                ",
                " j/k move  enter detail  d remove  tab agents  q quit           ",
            ]
        );
    }

    #[test]
    fn one_host_hides_the_host_column_and_offers_no_verbs() {
        let local = Host::Local {
            exe: "/opt/reef".into(),
            state: "/var/reef".into(),
        };
        let app = app(vec![state(local, None)]);
        assert_eq!(
            screen(&app, 40, 3),
            [
                " name   role   owner   desired   state  ",
                " connecting                             ",
                " j/k move  enter detail  tab roles  q q ",
            ]
        );
    }

    #[test]
    fn verbs_and_terminal_reach_the_detail_screen() {
        let rows = vec![agent("echo-1", State::Running, true, &[])];
        let mut app = app(vec![state(ssh("prod-eu"), Some(Ok(rows)))]);
        app.screen = Screen::Detail {
            row: Row::Agent(0, "echo-1".parse().unwrap()),
            detail: None,
            scroll: 0,
            polled: Instant::now(),
        };

        app.key(KeyCode::Char('u'));
        let (verb, row) = app.confirm.clone().expect("update asks from the detail");
        assert_eq!((verb.arg, row.name()), ("update", "echo-1"));

        app.key(KeyCode::Char('n'));
        assert!(app.confirm.is_none(), "any other key cancels");

        app.key(KeyCode::Char('t'));
        assert_eq!(app.shell.as_ref().map(Row::name), Some("echo-1"));

        app.key(KeyCode::Char('j'));
        assert!(
            matches!(app.screen, Screen::Detail { scroll: 1, .. }),
            "j still scrolls the detail"
        );
    }

    #[test]
    fn switching_views_refreshes_every_host_at_once() {
        let (wake, wakes) = mpsc::channel();
        let mut app = app(vec![HostState {
            host: ssh("prod-eu"),
            agents: None,
            roles: None,
            wake,
        }]);
        app.key(KeyCode::Tab);
        assert!(matches!(app.view, View::Roles));
        assert!(matches!(wakes.try_recv(), Ok(View::Roles)));
    }

    #[test]
    fn selection_follows_the_agent_across_refreshes() {
        let rows = || {
            vec![
                agent("echo-1", State::Running, true, &[]),
                agent("echo-2", State::Running, true, &[]),
            ]
        };
        let mut app = app(vec![
            state(ssh("prod-eu"), None),
            state(ssh("prod-us"), Some(Ok(rows()))),
        ]);
        app.selected = 2;
        assert_eq!(app.selected_row().unwrap().name(), "echo-2");
        app.apply(Msg::Agents(0, Ok(rows())));
        assert_eq!(app.selected, 3);
        assert_eq!(app.selected_row().unwrap().name(), "echo-2");
    }

    #[test]
    fn detail_shows_the_fields_and_the_event_tail() {
        let mut app = app(vec![state(ssh("prod-eu"), Some(Ok(vec![])))]);
        let detail = AgentDetail {
            name: "echo-1".parse().unwrap(),
            role: "echo".parse().unwrap(),
            role_digest: "0".repeat(64).parse().unwrap(),
            role_current: true,
            image: "alpine".parse().unwrap(),
            owner: "ana".to_owned(),
            fleet: false,
            resources: AgentResources {
                vcpus: 1,
                memory_mib: 512,
                disk_gib: None,
                max_pids: None,
            },
            egress: Vec::new(),
            secrets: BTreeMap::new(),
            volumes: BTreeMap::new(),
            desired: Desired::Running,
            state: State::Running,
            reason: None,
            generation: 1,
            applied_generation: 1,
            applied_digest: None,
            vm: Some(VmStatus::Running),
            sandbox: "reef-echo-1".to_owned(),
            ports: BTreeMap::from([("ui".parse().unwrap(), 19007)]),
            env: BTreeMap::new(),
        };
        let events = vec![Event {
            id: 4,
            agent: "echo-1".parse().unwrap(),
            at: now() - 180,
            kind: "create".to_owned(),
            detail: "sandbox reef-echo-1".to_owned(),
        }];
        app.screen = Screen::Detail {
            row: Row::Agent(0, "echo-1".parse().unwrap()),
            detail: Some(Ok(Detail::Agent(Box::new(detail), Ok(events)))),
            scroll: 0,
            polled: Instant::now(),
        };
        assert_eq!(
            screen(&app, 104, 7),
            [
                " agent echo-1                                               │  events                                   ",
                " ────────────                                               │  ──────                                   ",
                " name         echo-1                                        │  create       3m  sandbox reef-echo-1     ",
                " role         echo@000000000000                             │                                           ",
                " image        alpine                                        │                                           ",
                " owner        ana                                           │                                           ",
                " j/k scroll  esc back  s start  x stop  u update  d remove  t terminal  q quit                          ",
            ]
        );
    }

    fn tone(app: &App, width: u16, height: u16, column: &str, row: u16) -> (Color, Modifier) {
        let header = &screen(app, width, height)[0];
        let x = u16::try_from(header.find(column).unwrap()).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let cell = &terminal.backend().buffer()[(x, row)];
        (cell.fg, cell.modifier)
    }

    #[test]
    fn color_tracks_state_and_vm() {
        let app = app(vec![state(
            ssh("prod-eu"),
            Some(Ok(vec![
                agent("echo-1", State::Running, true, &[]),
                agent("echo-2", State::Failed, true, &[]),
                agent("echo-3", State::Pending, true, &[]),
            ])),
        )]);
        assert_eq!(tone(&app, 80, 6, "state", 1).0, Color::Green);
        assert_eq!(tone(&app, 80, 6, "state", 2).0, Color::Red);
        assert_eq!(tone(&app, 80, 6, "state", 3).0, Color::Yellow);
        assert_eq!(tone(&app, 80, 6, "vm", 1).0, Color::Green);
        assert_eq!(tone(&app, 80, 6, "vm", 2).0, Color::Reset);
    }

    #[test]
    fn no_color_keeps_failures_visible() {
        let mut app = app(vec![state(
            ssh("prod-eu"),
            Some(Ok(vec![
                agent("echo-1", State::Running, true, &[]),
                agent("echo-2", State::Failed, true, &[]),
            ])),
        )]);
        app.color = false;
        assert_eq!(tone(&app, 80, 5, "state", 1).0, Color::Reset);
        let (color, modifier) = tone(&app, 80, 5, "state", 2);
        assert_eq!(color, Color::Reset);
        assert!(modifier.contains(Modifier::BOLD));
    }
}
