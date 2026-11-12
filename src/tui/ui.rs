//! Desenho da TUI. Paleta: grafite + ciano (a cor do grimh3xx); amarelo e vermelho só para aviso e risco.

use super::{App, Tab, TABS};
use crate::catalog::{category_label, tr, RegValue, Risk};
use crate::repair::REPAIRS;
use crate::state::fmt_time;
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Cell, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap};
use ratatui::Frame;

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
const OK: Color = Color::Green;
const WARN: Color = Color::Yellow;
const BAD: Color = Color::Red;
const SPIN: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn risk_color(r: Risk) -> Color {
    match r {
        Risk::Safe => OK,
        Risk::Moderate => WARN,
        Risk::Advanced => BAD,
    }
}

fn panel(title: &str) -> Block<'static> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(DIM)).title(Span::styled(format!(" {title} "), Style::new().fg(ACCENT).bold()))
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [head, tabs, body, foot] = Layout::vertical([Constraint::Length(2), Constraint::Length(1), Constraint::Min(5), Constraint::Length(1)]).areas(f.area());
    header(f, head, app);

    let titles: Vec<Line> = TABS.iter().enumerate().map(|(i, t)| Line::from(format!("{} {}", i + 1, t.title()))).collect();
    let sel = TABS.iter().position(|t| *t == app.tab).unwrap_or(0);
    f.render_widget(Tabs::new(titles).select(sel).style(Style::new().fg(DIM)).highlight_style(Style::new().fg(ACCENT).bold().underlined()).divider("│"), tabs);

    match app.tab {
        Tab::Tweaks => tweaks(f, body, app),
        Tab::Apps => apps(f, body, app),
        Tab::Debloat => debloat(f, body, app),
        Tab::Profiles => profiles(f, body, app),
        Tab::Repairs => repairs(f, body, app),
        Tab::Log => log(f, body, app),
    }
    footer(f, foot, app);

    if let Some(c) = &app.confirm {
        let lines = c.text.lines().count() as u16;
        let area = centered(f.area(), 70, lines + 5);
        f.render_widget(Clear, area);
        let mut t = Text::from(c.text.as_str());
        t.lines.push(Line::default());
        t.lines.push(Line::from(vec![
            Span::styled(" Enter/s ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
            Span::raw(tr(" confirmar   ", " confirm   ")),
            Span::styled(" Esc ", Style::new().fg(Color::Black).bg(DIM)),
            Span::raw(tr(" cancelar", " cancel")),
        ]));
        f.render_widget(Paragraph::new(t).wrap(Wrap { trim: false }).block(panel(tr("Confirmar", "Confirm")).border_style(Style::new().fg(ACCENT))), area);
    }
}

fn centered(r: Rect, w: u16, h: u16) -> Rect {
    let [a] = Layout::horizontal([Constraint::Length(w.min(r.width))]).flex(Flex::Center).areas(r);
    let [a] = Layout::vertical([Constraint::Length(h.min(r.height))]).flex(Flex::Center).areas(a);
    a
}

fn header(f: &mut Frame, area: Rect, app: &App) {
    let mut l1 = vec![
        Span::styled(" ntos", Style::new().fg(Color::White).bold()),
        Span::styled("krnl", Style::new().fg(ACCENT).bold()),
        Span::styled(format!(" v{}  ", env!("CARGO_PKG_VERSION")), Style::new().fg(DIM)),
        Span::styled("by grimh3xx", Style::new().fg(DIM).italic()),
    ];
    // NTOSKRNL_DEMO: gravação do demo do README (sem o aviso de modo simulado)
    if !cfg!(windows) && std::env::var_os("NTOSKRNL_DEMO").is_none() {
        l1.push(Span::styled(tr("   [modo simulado: nada é alterado]", "   [simulated mode: nothing is changed]"), Style::new().fg(WARN)));
    }
    let l2 = match &app.info {
        Some(i) => Line::from(vec![
            Span::raw(" "),
            Span::styled(i.os_name(), Style::new().fg(Color::White)),
            Span::styled(format!("  ·  {}  ·  {}  ·  {} GB", i.cpu, i.gpus, i.ram_gb), Style::new().fg(DIM)),
        ]),
        None => Line::styled(tr(" lendo o PC...", " reading the PC..."), Style::new().fg(DIM)),
    };
    f.render_widget(Paragraph::new(vec![Line::from(l1), l2]), area);
}

fn footer(f: &mut Frame, area: Rect, app: &App) {
    let keys: &[(&'static str, &'static str)] = match app.tab {
        Tab::Tweaks => &[("espaço", "marcar"), ("a", "todos"), ("Enter", "aplicar"), ("u", "desfazer"), ("c", "categoria"), ("/", "buscar"), ("p", "ponto rest."), ("v", "conferir")],
        Tab::Apps => &[("espaço", "marcar"), ("a", "todos"), ("Enter", "instalar"), ("U", "atualizar tudo"), ("c", "categoria"), ("/", "buscar")],
        Tab::Debloat => &[("espaço", "marcar"), ("a", "todos"), ("Enter", "remover"), ("/", "buscar")],
        Tab::Profiles => &[("Enter", "aplicar"), ("u", "desfazer")],
        Tab::Repairs => &[("Enter", "rodar")],
        Tab::Log => &[("↑↓", "rolar"), ("End", "fim")],
    };
    fn en(k: &'static str) -> &'static str {
        match k {
        "espaço" => "space",
        "marcar" => "pick",
        "todos" => "all",
        "aplicar" => "apply",
        "desfazer" => "undo",
        "categoria" => "category",
        "buscar" => "search",
        "ponto rest." => "restore pt",
        "conferir" => "check",
        "instalar" => "install",
        "atualizar tudo" => "upgrade all",
        "remover" => "remove",
        "rodar" => "run",
        "rolar" => "scroll",
        "fim" => "end",
        x => x,
        }
    }
    let mut spans = Vec::new();
    if let Some(b) = &app.busy {
        spans.push(Span::styled(format!(" {} {b} ", SPIN[app.spin % SPIN.len()]), Style::new().fg(Color::Black).bg(ACCENT).bold()));
        spans.push(Span::raw(" "));
    } else if let Some(n) = &app.notice {
        spans.push(Span::styled(format!(" {n} "), Style::new().fg(Color::Black).bg(WARN)));
        spans.push(Span::raw(" "));
    }
    if app.filtering {
        spans.push(Span::styled(format!(" / {}█ ", app.filter), Style::new().fg(ACCENT).bold()));
    } else {
        for (k, v) in keys.iter().chain([("Tab", "aba"), ("q", "sair")].iter()) {
            spans.push(Span::styled(format!(" {} ", tr(k, en(k))), Style::new().fg(ACCENT).bold()));
            let v = match *v {
                "aba" => tr("aba", "tab"),
                "sair" => tr("sair", "quit"),
                x => tr(x, en(x)),
            };
            spans.push(Span::styled(format!("{v} "), Style::new().fg(DIM)));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Título da lista com filtro e categoria ativos
fn list_title(base: &str, app: &App, count: usize) -> String {
    let mut t = format!("{base} ({count})");
    if let Some(c) = &app.category {
        t += &format!(" · {}", category_label(c));
    }
    if !app.filter.is_empty() {
        t += &format!(" · \"{}\"", app.filter);
    }
    let picked = app.list().picked.len();
    if picked > 0 {
        t += &format!(" · {picked} {}", tr("marcado(s)", "picked"));
    }
    t
}

fn check(picked: bool) -> Span<'static> {
    if picked {
        Span::styled("[x]", Style::new().fg(ACCENT).bold())
    } else {
        Span::styled("[ ]", Style::new().fg(DIM))
    }
}

fn table_state(app: &App) -> TableState {
    TableState::default().with_selected(Some(app.list().cursor))
}

fn split(area: Rect) -> [Rect; 2] {
    if area.width >= 100 {
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(area)
    } else {
        Layout::vertical([Constraint::Percentage(60), Constraint::Percentage(40)]).areas(area)
    }
}

const HIGHLIGHT: Style = Style::new().bg(Color::Rgb(30, 42, 52)).add_modifier(Modifier::BOLD);

fn tweaks(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] = split(area);
    let vis = app.visible();
    let picked = &app.list().picked;
    let rows: Vec<Row> = vis
        .iter()
        .filter_map(|id| app.cat.tweak(id))
        .map(|t| {
            let s = app.status.get(&t.id);
            let blocked = s.is_some_and(|s| s.blocked.is_some());
            let state = match s {
                Some(s) if s.applied => Span::styled("●", Style::new().fg(OK)),
                Some(s) if s.revertible => Span::styled("◐", Style::new().fg(WARN)),
                _ if blocked => Span::styled("–", Style::new().fg(DIM)),
                _ => Span::styled("○", Style::new().fg(DIM)),
            };
            let title = Style::new().fg(if blocked { DIM } else { Color::White });
            Row::new(vec![
                Cell::from(check(picked.contains(&t.id))),
                Cell::from(state),
                Cell::from(Span::styled(t.title.get().to_string(), title)),
                Cell::from(Span::styled(category_label(&t.category), Style::new().fg(DIM))),
                Cell::from(Span::styled(t.risk.label(), Style::new().fg(risk_color(t.risk)))),
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(3), Constraint::Length(1), Constraint::Min(20), Constraint::Length(15), Constraint::Length(9)])
        .row_highlight_style(HIGHLIGHT)
        .highlight_symbol("▌")
        .block(panel(&list_title("Tweaks", app, vis.len())));
    f.render_stateful_widget(table, left, &mut table_state(app));

    let mut lines: Vec<Line> = Vec::new();
    if let Some(t) = app.current().and_then(|id| app.cat.tweak(&id)) {
        let s = app.status.get(&t.id);
        lines.push(Line::styled(t.title.get().to_string(), Style::new().fg(Color::White).bold()));
        lines.push(Line::from(vec![
            Span::styled(category_label(&t.category), Style::new().fg(DIM)),
            Span::raw("  ·  "),
            Span::styled(t.risk.label(), Style::new().fg(risk_color(t.risk))),
            Span::raw("  ·  "),
            match s {
                Some(s) if s.applied => Span::styled(tr("aplicado", "applied"), Style::new().fg(OK)),
                Some(s) if s.revertible => Span::styled(tr("aplicado, mas o Windows desfez parte", "applied, but Windows undid part of it"), Style::new().fg(WARN)),
                _ => Span::styled(tr("não aplicado", "not applied"), Style::new().fg(DIM)),
            },
        ]));
        if let Some(b) = s.and_then(|_| app.st_backup_time(&t.id)) {
            lines.push(Line::styled(format!("{} {}", tr("aplicado pelo ntoskrnl em", "applied by ntoskrnl on"), fmt_time(b)), Style::new().fg(DIM)));
        }
        lines.push(Line::default());
        if let Some(sm) = &t.summary {
            lines.push(Line::styled(sm.get().to_string(), Style::new().fg(ACCENT)));
            lines.push(Line::default());
        }
        lines.push(Line::raw(t.description.get().to_string()));
        if let Some(w) = &t.warning {
            lines.push(Line::default());
            lines.push(Line::styled(format!("⚠ {}", w.get()), Style::new().fg(WARN)));
        }
        if let Some(Some(b)) = s.map(|s| &s.blocked) {
            lines.push(Line::default());
            lines.push(Line::styled(format!("✕ {b}"), Style::new().fg(BAD)));
        }
        if t.restart {
            lines.push(Line::styled(tr("↻ precisa reiniciar para valer", "↻ needs a restart to take effect"), Style::new().fg(DIM)));
        }
        lines.push(Line::default());
        lines.push(Line::styled(tr("O que muda", "What changes"), Style::new().fg(ACCENT).bold()));
        for r in &t.reg {
            let v = match &r.value {
                RegValue::Num(n) => n.to_string(),
                RegValue::Str(s) => format!("\"{s}\""),
            };
            let before = app.st_original(&t.id, &r.path, &r.name).map(|o| format!("  ({} {o})", tr("antes:", "was:"))).unwrap_or_default();
            lines.push(Line::styled(format!("reg  {}\\{} = {v}{before}", r.path, r.name), Style::new().fg(DIM)));
        }
        for sv in &t.service {
            lines.push(Line::styled(format!("svc  {} → {:?}", sv.name, sv.start).to_lowercase(), Style::new().fg(DIM)));
        }
        for c in &t.cmd {
            lines.push(Line::styled(format!("ps   {} ({})", c.key, tr("script com backup e conferência", "script with backup and verify")), Style::new().fg(DIM)));
        }
    }
    let rp = if app.restore_point { tr("ponto de restauração: ligado", "restore point: on") } else { tr("ponto de restauração: desligado", "restore point: off") };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(tr("Detalhes", "Details")).title_bottom(Line::styled(format!(" {rp} "), Style::new().fg(DIM)).right_aligned())), right);
}

impl App {
    /// Valor original guardado no backup (só existe depois de aplicado)
    fn st_original(&self, id: &str, path: &str, name: &str) -> Option<String> {
        let st = self.st.try_lock().ok()?;
        st.backups.get(id)?.registry.iter().find(|x| x.same_key(path, name)).map(|x| x.display())
    }

    fn st_backup_time(&self, id: &str) -> Option<u64> {
        self.st.try_lock().ok().and_then(|s| s.backups.get(id).map(|b| b.applied_at))
    }
}

fn apps(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] = split(area);
    let vis = app.visible();
    let picked = &app.list().picked;
    let rows: Vec<Row> = vis
        .iter()
        .filter_map(|id| app.cat.apps.iter().find(|a| &a.id == id))
        .map(|a| {
            let inst = app.apps_installed.as_ref().map(|s| s.contains(&a.id.to_ascii_lowercase()));
            let mark = match inst {
                Some(true) => Span::styled("●", Style::new().fg(OK)),
                Some(false) => Span::styled("○", Style::new().fg(DIM)),
                None => Span::styled("·", Style::new().fg(DIM)),
            };
            Row::new(vec![
                Cell::from(check(picked.contains(&a.id))),
                Cell::from(mark),
                Cell::from(Span::styled(a.name.clone(), Style::new().fg(Color::White))),
                Cell::from(Span::styled(category_label(&a.category), Style::new().fg(DIM))),
            ])
        })
        .collect();
    let mut title = list_title(tr("Programas", "Apps"), app, vis.len());
    if app.apps_installed.is_none() {
        title += tr(" · lendo o winget...", " · reading winget...");
    }
    let table = Table::new(rows, [Constraint::Length(3), Constraint::Length(1), Constraint::Min(18), Constraint::Length(20)])
        .row_highlight_style(HIGHLIGHT)
        .highlight_symbol("▌")
        .block(panel(&title));
    f.render_stateful_widget(table, left, &mut table_state(app));

    let mut lines = Vec::new();
    if let Some(a) = app.current().and_then(|id| app.cat.apps.iter().find(|a| a.id == id)) {
        lines.push(Line::styled(a.name.clone(), Style::new().fg(Color::White).bold()));
        lines.push(Line::styled(category_label(&a.category), Style::new().fg(DIM)));
        lines.push(Line::default());
        lines.push(Line::raw(a.desc.get().to_string()));
        lines.push(Line::default());
        let src = a.source.as_deref().map(|s| format!(" --source {s}")).unwrap_or_default();
        lines.push(Line::styled(format!("winget install --id {} --exact{src}", a.id), Style::new().fg(DIM)));
        if app.apps_installed.as_ref().is_some_and(|s| s.contains(&a.id.to_ascii_lowercase())) {
            lines.push(Line::default());
            lines.push(Line::styled(tr("● já instalado (Enter reinstala/atualiza)", "● already installed (Enter reinstalls/updates)"), Style::new().fg(OK)));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(tr("Detalhes", "Details"))), right);
}

fn debloat(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] = split(area);
    let vis = app.visible();
    let picked = &app.list().picked;
    let rows: Vec<Row> = vis
        .iter()
        .filter_map(|id| app.cat.bloat.iter().find(|b| &b.id == id))
        .map(|b| {
            let inst = app.bloat_installed.as_ref().map(|s| s.contains(&b.id));
            let (mark, st) = match inst {
                Some(true) => (Span::styled("●", Style::new().fg(OK)), Style::new().fg(Color::White)),
                Some(false) => (Span::styled("–", Style::new().fg(DIM)), Style::new().fg(DIM)),
                None => (Span::styled("·", Style::new().fg(DIM)), Style::new().fg(Color::White)),
            };
            Row::new(vec![
                Cell::from(check(picked.contains(&b.id))),
                Cell::from(mark),
                Cell::from(Span::styled(b.name.clone(), st)),
                Cell::from(if b.caution.is_some() { Span::styled("⚠", Style::new().fg(WARN)) } else { Span::raw("") }),
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(3), Constraint::Length(1), Constraint::Min(18), Constraint::Length(2)])
        .row_highlight_style(HIGHLIGHT)
        .highlight_symbol("▌")
        .block(panel(&list_title(tr("Apps pré-instalados", "Preinstalled apps"), app, vis.len())));
    f.render_stateful_widget(table, left, &mut table_state(app));

    let mut lines = Vec::new();
    if let Some(b) = app.current().and_then(|id| app.cat.bloat.iter().find(|b| b.id == id)) {
        lines.push(Line::styled(b.name.clone(), Style::new().fg(Color::White).bold()));
        lines.push(Line::styled(b.id.clone(), Style::new().fg(DIM)));
        lines.push(Line::default());
        lines.push(Line::raw(b.desc.get().to_string()));
        if let Some(c) = &b.caution {
            lines.push(Line::default());
            lines.push(Line::styled(format!("⚠ {}", c.get()), Style::new().fg(WARN)));
        }
        if app.bloat_installed.as_ref().is_some_and(|s| !s.contains(&b.id)) {
            lines.push(Line::default());
            lines.push(Line::styled(tr("Não está instalado neste PC.", "Not installed on this PC."), Style::new().fg(DIM)));
        }
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        tr("Lista fechada: nada fora dela é removido, e nada que os jogos usam (Xbox Identity, Store, Game Bar) está aqui.", "Closed list: nothing outside it is removed, and nothing games need (Xbox Identity, Store, Game Bar) is in it."),
        Style::new().fg(DIM),
    ));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(tr("Detalhes", "Details"))), right);
}

fn profiles(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] = split(area);
    let rows: Vec<Row> = app
        .cat
        .profiles
        .iter()
        .map(|p| {
            let done = p.tweaks.iter().filter(|id| app.status.get(*id).is_some_and(|s| s.applied)).count();
            let fit = p.tweaks.iter().filter(|id| app.status.get(*id).is_none_or(|s| s.blocked.is_none())).count();
            Row::new(vec![
                Cell::from(Span::styled(p.title.get().to_string(), Style::new().fg(Color::White).bold())),
                Cell::from(Span::styled(format!("{done}/{fit}"), Style::new().fg(if done == fit && fit > 0 { OK } else { DIM }))),
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Min(16), Constraint::Length(7)]).row_highlight_style(HIGHLIGHT).highlight_symbol("▌").block(panel(tr("Perfis", "Profiles")));
    f.render_stateful_widget(table, left, &mut table_state(app));

    let mut lines = Vec::new();
    if let Some(p) = app.current().and_then(|id| app.cat.profile(&id)) {
        lines.push(Line::styled(p.title.get().to_string(), Style::new().fg(Color::White).bold()));
        lines.push(Line::default());
        lines.push(Line::raw(p.description.get().to_string()));
        lines.push(Line::default());
        lines.push(Line::styled("Tweaks", Style::new().fg(ACCENT).bold()));
        for id in &p.tweaks {
            let Some(t) = app.cat.tweak(id) else { continue };
            let s = app.status.get(id);
            let (mark, st) = match s {
                Some(s) if s.blocked.is_some() => ("–", Style::new().fg(DIM)),
                Some(s) if s.applied => ("●", Style::new().fg(OK)),
                _ => ("○", Style::new().fg(Color::White)),
            };
            lines.push(Line::from(vec![Span::styled(format!("{mark} "), st), Span::styled(t.title.get().to_string(), st)]));
        }
        if !p.debloat.is_empty() {
            lines.push(Line::default());
            lines.push(Line::styled(tr("Remove", "Removes"), Style::new().fg(ACCENT).bold()));
            let names: Vec<_> = p.debloat.iter().filter_map(|id| app.cat.bloat.iter().find(|b| &b.id == id)).map(|b| b.name.as_str()).collect();
            lines.push(Line::styled(names.join(", "), Style::new().fg(DIM)));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(tr("Detalhes", "Details"))), right);
}

fn repairs(f: &mut Frame, area: Rect, app: &App) {
    let [left, right] = split(area);
    let rows: Vec<Row> = REPAIRS.iter().map(|r| Row::new(vec![Cell::from(Span::styled(r.title(), Style::new().fg(Color::White)))])).collect();
    let table = Table::new(rows, [Constraint::Min(20)]).row_highlight_style(HIGHLIGHT).highlight_symbol("▌").block(panel(tr("Reparos", "Repairs")));
    f.render_stateful_widget(table, left, &mut table_state(app));
    let mut lines = Vec::new();
    if let Some(r) = REPAIRS.get(app.list().cursor) {
        lines.push(Line::styled(r.title(), Style::new().fg(Color::White).bold()));
        lines.push(Line::default());
        lines.push(Line::raw(r.description()));
        if r.restart {
            lines.push(Line::default());
            lines.push(Line::styled(tr("↻ precisa reiniciar depois", "↻ needs a restart afterwards"), Style::new().fg(DIM)));
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(tr("Detalhes", "Details"))), right);
}

fn log(f: &mut Frame, area: Rect, app: &App) {
    let h = area.height.saturating_sub(2) as usize;
    let end = app.log.len().saturating_sub(app.log_scroll);
    let start = end.saturating_sub(h);
    let lines: Vec<Line> = app.log[start..end]
        .iter()
        .map(|l| {
            let t = l.trim_start();
            let st = if t == "ok" || t.starts_with("ok ") {
                Style::new().fg(OK)
            } else if t.starts_with(tr("falhou", "failed")) || t.starts_with("ntoskrnl:") {
                Style::new().fg(BAD)
            } else if l.starts_with("  ") {
                Style::new().fg(DIM)
            } else {
                Style::new().fg(Color::White)
            };
            Line::styled(l.clone(), st)
        })
        .collect();
    let title = if app.log_scroll > 0 { format!("Log (↑{})", app.log_scroll) } else { "Log".into() };
    f.render_widget(Paragraph::new(lines).block(panel(&title)), area);
}
