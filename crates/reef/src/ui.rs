mod host;

pub use host::Alias;

use crate::rows::{AgentDetail, AgentRow};
use anyhow::{Context, Result};
use host::{Failure, Host};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use reef_core::{AgentName, State, VmStatus};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(5);
const PLAIN: Style = Style::new();
const DIM: Style = Style::new().add_modifier(Modifier::DIM);
const RED: Style = Style::new().fg(Color::Red);
const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);
const COLUMNS: [&str; 9] = [
    "host", "name", "role", "owner", "desired", "state", "vm", "sync", "ports",
];

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
    let mut app = App::new(hosts, tx);
    let mut terminal = ratatui::try_init()?;
    let outcome = app.drive(&mut terminal, &rx);
    ratatui::restore();
    outcome
}

#[derive(Clone, Copy)]
enum Verb {
    Start,
    Stop,
    Update,
    Remove,
}

impl Verb {
    fn arg(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Update => "update",
            Self::Remove => "rm",
        }
    }

    fn progress(self) -> &'static str {
        match self {
            Self::Start => "starting",
            Self::Stop => "stopping",
            Self::Update => "updating",
            Self::Remove => "removing",
        }
    }
}

enum Msg {
    Agents(usize, Result<Vec<AgentRow>, Failure>),
    Detail(usize, AgentName, Result<Box<AgentDetail>, Failure>),
    Done(usize, AgentName, Result<(), Failure>),
}

struct HostState {
    host: Host,
    agents: Option<Result<Vec<AgentRow>, Failure>>,
    busy: BTreeMap<AgentName, Verb>,
    wake: Sender<()>,
}

enum Screen {
    Table,
    Detail {
        host: usize,
        name: AgentName,
        detail: Option<Result<Box<AgentDetail>, Failure>>,
        scroll: u16,
    },
    Confirm(Verb, usize, AgentName),
}

enum Item<'a> {
    Agent(usize, &'a AgentRow),
    Host(usize, String, Style),
}

struct App {
    hosts: Vec<HostState>,
    selected: usize,
    screen: Screen,
    flash: Option<String>,
    quit: bool,
    tx: Sender<Msg>,
}

impl App {
    fn new(hosts: Vec<Host>, tx: Sender<Msg>) -> Self {
        let hosts = hosts
            .into_iter()
            .enumerate()
            .map(|(index, host)| {
                let (wake, wakes) = mpsc::channel();
                poll(index, host.clone(), tx.clone(), wakes);
                HostState {
                    host,
                    agents: None,
                    busy: BTreeMap::new(),
                    wake,
                }
            })
            .collect();
        Self {
            hosts,
            selected: 0,
            screen: Screen::Table,
            flash: None,
            quit: false,
            tx,
        }
    }

    fn drive(&mut self, terminal: &mut DefaultTerminal, rx: &Receiver<Msg>) -> Result<()> {
        while !self.quit {
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(50))?
                && let Event::Key(key) = event::read()?
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
        }
        Ok(())
    }

    fn items(&self) -> Vec<Item<'_>> {
        let mut items = Vec::new();
        for (index, host) in self.hosts.iter().enumerate() {
            match &host.agents {
                None => items.push(Item::Host(index, "connecting".to_owned(), DIM)),
                Some(Err(failure)) => items.push(Item::Host(index, failure.to_string(), RED)),
                Some(Ok(agents)) if agents.is_empty() => {
                    items.push(Item::Host(index, "no agents".to_owned(), DIM));
                }
                Some(Ok(agents)) => {
                    items.extend(agents.iter().map(|agent| Item::Agent(index, agent)));
                }
            }
        }
        items
    }

    fn selected_agent(&self) -> Option<(usize, AgentName)> {
        match self.items().get(self.selected)? {
            Item::Agent(index, agent) => Some((*index, agent.name.clone())),
            Item::Host(..) => None,
        }
    }

    fn position(&self, host: usize, name: &AgentName) -> Option<usize> {
        self.items()
            .iter()
            .position(|item| matches!(item, Item::Agent(index, agent) if *index == host && agent.name == *name))
    }

    fn key(&mut self, code: KeyCode) {
        self.flash = None;
        match &mut self.screen {
            Screen::Confirm(verb, index, name) => {
                let (verb, target) = (*verb, (*index, name.clone()));
                self.screen = Screen::Table;
                if code == KeyCode::Char('y') {
                    self.act(verb, target);
                }
            }
            Screen::Detail { scroll, .. } => match code {
                KeyCode::Esc => self.screen = Screen::Table,
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
                _ => {}
            },
            Screen::Table => match code {
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.selected = self.clamp(self.selected + 1),
                KeyCode::Enter => self.open(),
                KeyCode::Char('s') => self.act_selected(Verb::Start),
                KeyCode::Char('x') => self.act_selected(Verb::Stop),
                KeyCode::Char('u') => self.confirm(Verb::Update),
                KeyCode::Char('d') => self.confirm(Verb::Remove),
                _ => {}
            },
        }
    }

    fn clamp(&self, index: usize) -> usize {
        index.min(self.items().len().saturating_sub(1))
    }

    fn confirm(&mut self, verb: Verb) {
        if let Some((index, name)) = self.selected_agent() {
            self.screen = Screen::Confirm(verb, index, name);
        }
    }

    fn act_selected(&mut self, verb: Verb) {
        if let Some(target) = self.selected_agent() {
            self.act(verb, target);
        }
    }

    fn open(&mut self) {
        let Some((host, name)) = self.selected_agent() else {
            return;
        };
        let target = self.hosts[host].host.clone();
        let tx = self.tx.clone();
        let agent = name.clone();
        thread::spawn(move || {
            let detail = target
                .fetch(&["agent", "get", agent.as_str(), "--json"])
                .map(Box::new);
            tx.send(Msg::Detail(host, agent, detail)).ok();
        });
        self.screen = Screen::Detail {
            host,
            name,
            detail: None,
            scroll: 0,
        };
    }

    fn act(&mut self, verb: Verb, (index, name): (usize, AgentName)) {
        let state = &mut self.hosts[index];
        if state.busy.contains_key(&name) {
            self.flash = Some(format!("{name} is busy"));
            return;
        }
        state.busy.insert(name.clone(), verb);
        let host = state.host.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let outcome = host.run(&["agent", verb.arg(), name.as_str()]);
            tx.send(Msg::Done(index, name, outcome)).ok();
        });
    }

    fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Agents(index, agents) => {
                let anchor = self.selected_agent();
                self.hosts[index].agents = Some(agents);
                self.selected = anchor
                    .and_then(|(host, name)| self.position(host, &name))
                    .unwrap_or_else(|| self.clamp(self.selected));
            }
            Msg::Detail(index, agent, fetched) => {
                if let Screen::Detail {
                    host, name, detail, ..
                } = &mut self.screen
                    && *host == index
                    && *name == agent
                {
                    *detail = Some(fetched);
                }
            }
            Msg::Done(index, name, outcome) => {
                let host = &mut self.hosts[index];
                host.busy.remove(&name);
                host.wake.send(()).ok();
                if let Err(failure) = outcome {
                    self.flash = Some(format!("{name}: {failure}"));
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let [body, footer] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
        match &self.screen {
            Screen::Detail {
                host,
                name,
                detail,
                scroll,
            } => frame.render_widget(
                self.detail(*host, name, detail.as_ref())
                    .scroll((*scroll, 0)),
                body,
            ),
            _ => frame.render_widget(self.table(body), body),
        }
        frame.render_widget(self.footer(), footer);
    }

    fn table(&self, area: Rect) -> Paragraph<'static> {
        let items = self.items();
        let first = usize::from(self.hosts.len() == 1);
        let cells: Vec<[String; 9]> = items
            .iter()
            .filter_map(|item| match item {
                Item::Agent(index, agent) => Some(self.cells(*index, agent)),
                Item::Host(..) => None,
            })
            .collect();
        let widths: Vec<usize> = (0..COLUMNS.len())
            .map(|column| {
                cells
                    .iter()
                    .map(|row| row[column].chars().count())
                    .chain([COLUMNS[column].len()])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let header = COLUMNS[first..]
            .iter()
            .zip(&widths[first..])
            .map(|(name, width)| Span::styled(pad(name, *width), DIM))
            .collect::<Vec<_>>();
        let visible = usize::from(area.height.saturating_sub(1));
        let offset = self.selected.saturating_sub(visible.saturating_sub(1));
        let mut lines = vec![Line::from(header)];
        for (position, item) in items.iter().enumerate().skip(offset) {
            let spans = match item {
                Item::Agent(index, agent) => self.cells(*index, agent)[first..]
                    .iter()
                    .zip(&widths[first..])
                    .enumerate()
                    .map(|(column, (text, width))| {
                        let style = match COLUMNS[first + column] {
                            "host" | "owner" => DIM,
                            "state" if agent.state == State::Failed => RED,
                            _ => PLAIN,
                        };
                        Span::styled(pad(text, *width), style)
                    })
                    .collect(),
                Item::Host(index, status, style) => [
                    Span::styled(pad(self.hosts[*index].host.label(), widths[0]), DIM),
                    Span::styled(status.clone(), *style),
                ][first..]
                    .to_vec(),
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

    fn cells(&self, index: usize, agent: &AgentRow) -> [String; 9] {
        let host = &self.hosts[index];
        let state = host
            .busy
            .get(&agent.name)
            .map_or(agent.state.label(), |verb| verb.progress());
        let role = if agent.role_current {
            agent.role.to_string()
        } else {
            format!("{} stale", agent.role)
        };
        let ports: Vec<String> = agent
            .ports
            .iter()
            .map(|(name, port)| format!("{name}={port}"))
            .collect();
        [
            host.host.label().to_owned(),
            agent.name.to_string(),
            role,
            agent.owner.clone(),
            agent.desired.label().to_owned(),
            state.to_owned(),
            agent.vm.map_or("-", VmStatus::label).to_owned(),
            if agent.synced { "yes" } else { "drift" }.to_owned(),
            ports.join(" "),
        ]
    }

    fn detail(
        &self,
        index: usize,
        name: &AgentName,
        detail: Option<&Result<Box<AgentDetail>, Failure>>,
    ) -> Paragraph<'static> {
        let host = &self.hosts[index].host;
        let lines = match detail {
            None => vec![
                field("name", name.to_string(), PLAIN),
                field("state", "loading".to_owned(), DIM),
            ],
            Some(Err(failure)) => vec![
                field("name", name.to_string(), PLAIN),
                field("error", failure.to_string(), RED),
            ],
            Some(Ok(detail)) => {
                let mut lines: Vec<Line<'static>> = detail
                    .rows()
                    .into_iter()
                    .map(|(label, value)| {
                        let style = match label {
                            "state" if detail.reason.is_some() => RED,
                            _ => PLAIN,
                        };
                        field(label, value, style)
                    })
                    .collect();
                lines.extend(
                    detail
                        .ports
                        .values()
                        .filter_map(|port| host.forward(*port))
                        .map(|forward| field("forward", forward, DIM)),
                );
                lines.push(field("terminal", host.terminal(name), DIM));
                lines
            }
        };
        Paragraph::new(lines).wrap(Wrap { trim: false })
    }

    fn footer(&self) -> Line<'static> {
        if let Some(flash) = &self.flash {
            return Line::styled(flash.clone(), RED);
        }
        match &self.screen {
            Screen::Table => Line::styled(
                "j/k move  enter detail  s start  x stop  u update  d remove  q quit",
                DIM,
            ),
            Screen::Detail { .. } => Line::styled("j/k scroll  esc back  q quit", DIM),
            Screen::Confirm(verb, index, name) => Line::from(format!(
                "{} {name} on {}? y/n",
                verb.arg(),
                self.hosts[*index].host.label()
            )),
        }
    }
}

fn poll(index: usize, host: Host, tx: Sender<Msg>, wake: Receiver<()>) {
    thread::spawn(move || {
        loop {
            let agents = host.fetch(&["agent", "list", "--json"]);
            if tx.send(Msg::Agents(index, agents)).is_err()
                || wake.recv_timeout(POLL) == Err(RecvTimeoutError::Disconnected)
            {
                return;
            }
        }
    });
}

fn field(label: &str, value: String, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(pad(label, 10), DIM),
        Span::styled(value, style),
    ])
}

fn pad(text: &str, width: usize) -> String {
    format!("{text:<width$} ")
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

    fn state(host: Host, agents: Option<Result<Vec<AgentRow>, Failure>>) -> HostState {
        HostState {
            host,
            agents,
            busy: BTreeMap::new(),
            wake: mpsc::channel().0,
        }
    }

    fn app(hosts: Vec<HostState>) -> App {
        let (tx, _) = mpsc::channel();
        App {
            hosts,
            selected: 0,
            screen: Screen::Table,
            flash: None,
            quit: false,
            tx,
        }
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
        app.hosts[0]
            .busy
            .insert("echo-2".parse().unwrap(), Verb::Update);
        assert_eq!(
            screen(&app, 80, 6),
            [
                "host    name   role       owner desired state    vm      sync  ports            ",
                "prod-eu echo-1 echo       ana   running running  running yes   ui=19007         ",
                "prod-eu echo-2 echo stale ana   running updating -       drift                  ",
                "prod-us unreachable: ssh: connect refused                                       ",
                "                                                                                ",
                "j/k move  enter detail  s start  x stop  u update  d remove  q quit             ",
            ]
        );
    }

    #[test]
    fn one_host_hides_the_host_column() {
        let local = Host::Local {
            exe: "/opt/reef".into(),
            state: "/var/reef".into(),
        };
        let app = app(vec![state(local, None)]);
        assert_eq!(
            screen(&app, 40, 3),
            [
                "name role owner desired state vm sync po",
                "connecting                              ",
                "j/k move  enter detail  s start  x stop ",
            ]
        );
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
        assert_eq!(app.selected_agent().unwrap().1.as_str(), "echo-2");
        app.apply(Msg::Agents(0, Ok(rows())));
        assert_eq!(app.selected, 3);
        assert_eq!(app.selected_agent().unwrap().1.as_str(), "echo-2");
    }

    #[test]
    fn detail_prints_the_commands_to_type() {
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
        app.screen = Screen::Detail {
            host: 0,
            name: "echo-1".parse().unwrap(),
            detail: Some(Ok(Box::new(detail))),
            scroll: 10,
        };
        assert_eq!(
            screen(&app, 60, 6),
            [
                "ports      ui=http://echo-1.localhost:19007                 ",
                "synced     yes                                              ",
                "forward    ssh -N -L 19007:127.0.0.1:19007 -- prod-eu       ",
                "terminal   ssh -t -- prod-eu reef agent ssh echo-1          ",
                "                                                            ",
                "j/k scroll  esc back  q quit                                ",
            ]
        );
    }
}
