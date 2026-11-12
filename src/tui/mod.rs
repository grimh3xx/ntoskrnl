//! TUI: estado, teclas e o worker. Tudo que demora (PowerShell, winget, sfc) roda numa thread e manda
//! mensagens de volta, então a tela nunca congela.

mod ui;

use crate::catalog::{tr, Catalog};
use crate::engine::{self, TweakStatus};
use crate::state::State;
use crate::sys::{self, SysInfo};
use crate::{apps, repair};
use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Tweaks,
    Apps,
    Debloat,
    Profiles,
    Repairs,
    Log,
}

pub const TABS: [Tab; 6] = [Tab::Tweaks, Tab::Apps, Tab::Debloat, Tab::Profiles, Tab::Repairs, Tab::Log];

impl Tab {
    pub fn title(self) -> &'static str {
        match self {
            Tab::Tweaks => "Tweaks",
            Tab::Apps => tr("Programas", "Apps"),
            Tab::Debloat => "Debloat",
            Tab::Profiles => tr("Perfis", "Profiles"),
            Tab::Repairs => tr("Reparos", "Repairs"),
            Tab::Log => "Log",
        }
    }
}

#[derive(Clone)]
enum Job {
    Init,
    Apply(Vec<String>, bool),
    Revert(Vec<String>),
    Profile(String, bool),
    ProfileRevert(String),
    Install(Vec<String>),
    Upgrade,
    Debloat(Vec<String>),
    Repair(String),
    Check,
}

enum Msg {
    Info(SysInfo),
    Status(BTreeMap<String, TweakStatus>),
    Apps(HashSet<String>),
    Bloat(HashSet<String>),
    Drift(Vec<String>),
    Log(String),
    Done(String),
}

pub struct Confirm {
    pub text: String,
    job: Job,
}

/// Cursor e seleção de uma lista
#[derive(Default)]
pub struct ListState {
    pub cursor: usize,
    pub picked: HashSet<String>,
}

pub struct App {
    pub cat: Arc<Catalog>,
    pub tab: Tab,
    pub info: Option<SysInfo>,
    pub status: BTreeMap<String, TweakStatus>,
    pub apps_installed: Option<HashSet<String>>,
    pub bloat_installed: Option<HashSet<String>>,
    pub drift: Vec<String>,
    pub lists: [ListState; 6],
    pub filter: String,
    pub filtering: bool,
    /// categoria filtrada em Tweaks/Programas (None = todas)
    pub category: Option<String>,
    pub restore_point: bool,
    pub confirm: Option<Confirm>,
    pub busy: Option<String>,
    pub spin: usize,
    pub log: Vec<String>,
    pub log_scroll: usize,
    pub notice: Option<String>,
    quit: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    st: Arc<Mutex<State>>,
    sys: Arc<Mutex<SysInfo>>,
}

pub fn run(cat: Catalog, restore_point: bool) -> Result<()> {
    let mut app = App::new(cat, restore_point);
    app.start(Job::Init, tr("lendo o PC", "reading the PC"));

    let mut term = ratatui::init();
    let res = (|| -> Result<()> {
        while !app.quit {
            term.draw(|f| ui::draw(f, &mut app))?;
            if event::poll(Duration::from_millis(80))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind == KeyEventKind::Press {
                        app.key(k);
                    }
                }
            }
            while let Ok(m) = app.rx.try_recv() {
                app.handle(m);
            }
            app.spin = app.spin.wrapping_add(1);
        }
        Ok(())
    })();
    ratatui::restore();
    res
}

impl App {
    fn new(cat: Catalog, restore_point: bool) -> Self {
        let (tx, rx) = channel();
        App {
            cat: Arc::new(cat),
            tab: Tab::Tweaks,
            info: None,
            status: BTreeMap::new(),
            apps_installed: None,
            bloat_installed: None,
            drift: Vec::new(),
            lists: Default::default(),
            filter: String::new(),
            filtering: false,
            category: None,
            restore_point,
            confirm: None,
            busy: None,
            spin: 0,
            log: Vec::new(),
            log_scroll: 0,
            notice: None,
            quit: false,
            tx,
            rx,
            st: Arc::new(Mutex::new(State::load())),
            sys: Arc::new(Mutex::new(SysInfo::default())),
        }
    }

    pub fn list(&self) -> &ListState {
        &self.lists[self.tab as usize]
    }

    fn list_mut(&mut self) -> &mut ListState {
        &mut self.lists[self.tab as usize]
    }

    /// Ids visíveis na aba atual, já com filtro de texto e de categoria
    pub fn visible(&self) -> Vec<String> {
        let f = self.filter.to_lowercase();
        let hit = |s: &[&str]| f.is_empty() || s.iter().any(|x| x.to_lowercase().contains(&f));
        let cat_ok = |c: &str| self.category.as_deref().is_none_or(|x| x == c);
        match self.tab {
            Tab::Tweaks => self
                .cat
                .tweaks
                .iter()
                .filter(|t| cat_ok(&t.category) && hit(&[&t.id, t.title.get(), t.description.get()]))
                .map(|t| t.id.clone())
                .collect(),
            Tab::Apps => self.cat.apps.iter().filter(|a| cat_ok(&a.category) && hit(&[&a.id, &a.name, a.desc.get()])).map(|a| a.id.clone()).collect(),
            Tab::Debloat => self.cat.bloat.iter().filter(|b| hit(&[&b.id, &b.name])).map(|b| b.id.clone()).collect(),
            Tab::Profiles => self.cat.profiles.iter().map(|p| p.id.clone()).collect(),
            Tab::Repairs => repair::REPAIRS.iter().map(|r| r.id.to_string()).collect(),
            Tab::Log => Vec::new(),
        }
    }

    pub fn current(&self) -> Option<String> {
        self.visible().get(self.list().cursor).cloned()
    }

    /// Categorias da aba na ordem do catálogo
    pub fn categories(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        let all: Vec<&str> = match self.tab {
            Tab::Tweaks => self.cat.tweaks.iter().map(|t| t.category.as_str()).collect(),
            Tab::Apps => self.cat.apps.iter().map(|a| a.category.as_str()).collect(),
            _ => Vec::new(),
        };
        for c in all {
            if !v.iter().any(|x| x == c) {
                v.push(c.to_string());
            }
        }
        v
    }

    fn start(&mut self, job: Job, label: &str) {
        if self.busy.is_some() {
            self.notice = Some(tr("Espere a tarefa atual terminar", "Wait for the current task to finish").into());
            return;
        }
        self.busy = Some(label.to_string());
        let (tx, cat, st, sys) = (self.tx.clone(), self.cat.clone(), self.st.clone(), self.sys.clone());
        std::thread::spawn(move || {
            let tx2 = tx.clone();
            let mut log = move |l: String| {
                let _ = tx2.send(Msg::Log(l));
            };
            let summary = work(job, &cat, &st, &sys, &tx, &mut log);
            let _ = tx.send(Msg::Done(summary));
        });
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Info(i) => self.info = Some(i),
            Msg::Status(s) => self.status = s,
            Msg::Apps(a) => self.apps_installed = Some(a),
            Msg::Bloat(b) => self.bloat_installed = Some(b),
            Msg::Drift(d) => {
                if !d.is_empty() {
                    self.notice = Some(format!(
                        "{} {} {}",
                        tr("O Windows desfez", "Windows undid"),
                        d.len(),
                        tr("tweak(s): tecle v em Tweaks para reaplicar", "tweak(s): press v in Tweaks to reapply")
                    ));
                }
                self.drift = d;
            }
            Msg::Log(l) => {
                self.log.push(l);
                if self.log.len() > 5000 {
                    self.log.drain(..1000);
                }
            }
            Msg::Done(s) => {
                self.busy = None;
                if !s.is_empty() {
                    self.notice = Some(s);
                }
                // o que acabou de ser feito fica desmarcado
                for l in &mut self.lists {
                    l.picked.clear();
                }
            }
        }
    }

    /// Selecionados; sem nenhum marcado, vale o item do cursor
    fn targets(&self) -> Vec<String> {
        let vis = self.visible();
        let picked = &self.list().picked;
        if picked.is_empty() {
            return self.current().into_iter().collect();
        }
        // na ordem do catálogo, não na ordem em que foram marcados
        let order: Vec<String> = match self.tab {
            Tab::Tweaks => self.cat.tweaks.iter().map(|t| t.id.clone()).collect(),
            Tab::Apps => self.cat.apps.iter().map(|a| a.id.clone()).collect(),
            Tab::Debloat => self.cat.bloat.iter().map(|b| b.id.clone()).collect(),
            _ => vis,
        };
        order.into_iter().filter(|id| picked.contains(id)).collect()
    }

    fn ask(&mut self, text: String, job: Job) {
        self.confirm = Some(Confirm { text, job });
    }

    fn key(&mut self, k: KeyEvent) {
        self.notice = None;
        if let Some(c) = self.confirm.take() {
            match k.code {
                KeyCode::Char('s' | 'S' | 'y' | 'Y') | KeyCode::Enter => {
                    let label = c.text.lines().next().unwrap_or("").trim_end_matches('?').to_string();
                    self.start(c.job, &label);
                    self.tab = Tab::Log;
                    self.log_scroll = 0;
                }
                _ => {}
            }
            return;
        }
        if self.filtering {
            match k.code {
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filtering = false;
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(ch) => self.filter.push(ch),
                _ => {}
            }
            self.list_mut().cursor = 0;
            return;
        }

        let n = self.visible().len();
        match k.code {
            KeyCode::Char('q') => {
                if self.busy.is_some() {
                    self.notice = Some(tr("Tem uma tarefa rodando: espere terminar para sair", "A task is running: wait for it before quitting").into());
                } else {
                    self.quit = true;
                }
            }
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.busy.is_none() {
                    self.quit = true;
                }
            }
            KeyCode::Tab | KeyCode::Right => self.switch(1),
            KeyCode::BackTab | KeyCode::Left => self.switch(TABS.len() - 1),
            KeyCode::Char(d @ '1'..='6') => self.goto(TABS[d as usize - '1' as usize]),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-1, n),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(1, n),
            KeyCode::PageUp => self.move_by(-10, n),
            KeyCode::PageDown => self.move_by(10, n),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(-100_000, n),
            KeyCode::End | KeyCode::Char('G') => self.move_by(100_000, n),
            KeyCode::Char('/') if matches!(self.tab, Tab::Tweaks | Tab::Apps | Tab::Debloat) => {
                self.filtering = true;
                self.filter.clear();
            }
            KeyCode::Esc => {
                self.filter.clear();
                self.category = None;
                self.list_mut().picked.clear();
            }
            KeyCode::Char(' ') if matches!(self.tab, Tab::Tweaks | Tab::Apps | Tab::Debloat) => {
                if let Some(id) = self.current() {
                    if self.selectable(&id) {
                        let p = &mut self.list_mut().picked;
                        if !p.remove(&id) {
                            p.insert(id);
                        }
                    }
                    self.move_by(1, n);
                }
            }
            KeyCode::Char('a') if matches!(self.tab, Tab::Tweaks | Tab::Apps | Tab::Debloat) => {
                let vis: Vec<_> = self.visible().into_iter().filter(|id| self.selectable(id)).collect();
                let p = &mut self.list_mut().picked;
                if vis.iter().all(|id| p.contains(id)) {
                    vis.iter().for_each(|id| {
                        p.remove(id);
                    });
                } else {
                    p.extend(vis);
                }
            }
            KeyCode::Char('c') if matches!(self.tab, Tab::Tweaks | Tab::Apps) => {
                let cats = self.categories();
                let i = self.category.as_ref().and_then(|c| cats.iter().position(|x| x == c));
                self.category = match i {
                    None => cats.first().cloned(),
                    Some(i) if i + 1 < cats.len() => Some(cats[i + 1].clone()),
                    _ => None,
                };
                self.list_mut().cursor = 0;
            }
            KeyCode::Char('p') if self.tab == Tab::Tweaks => self.restore_point = !self.restore_point,
            KeyCode::Enter => self.action(),
            KeyCode::Char('u') => self.undo(),
            KeyCode::Char('U') if self.tab == Tab::Apps => self.ask(tr("Atualizar todos os programas pelo winget?", "Upgrade every app with winget?").into(), Job::Upgrade),
            KeyCode::Char('v') if self.tab == Tab::Tweaks => {
                if self.drift.is_empty() {
                    self.start(Job::Check, tr("conferindo tweaks", "checking tweaks"));
                } else {
                    let names = self.titles(&self.drift.clone());
                    let text = format!("{}\n\n{names}", tr("Reaplicar o que o Windows desfez?", "Reapply what Windows undid?"));
                    self.ask(text, Job::Apply(self.drift.clone(), false));
                }
            }
            _ => {}
        }
    }

    fn selectable(&self, id: &str) -> bool {
        match self.tab {
            Tab::Tweaks => self.status.get(id).is_none_or(|s| s.blocked.is_none()),
            Tab::Debloat => self.bloat_installed.as_ref().is_none_or(|b| b.contains(id)),
            _ => true,
        }
    }

    fn titles(&self, ids: &[String]) -> String {
        ids.iter().map(|id| format!("  · {}", self.cat.tweak(id).map_or(id.as_str(), |t| t.title.get()))).collect::<Vec<_>>().join("\n")
    }

    fn action(&mut self) {
        let ids = self.targets();
        match self.tab {
            Tab::Tweaks => {
                let ids: Vec<_> = ids.into_iter().filter(|id| self.selectable(id)).collect();
                if ids.is_empty() {
                    self.notice = Some(tr("Esse tweak não serve para este PC", "That tweak does not fit this PC").into());
                    return;
                }
                let rp = if self.restore_point { tr("\n\nAntes: ponto de restauração (p desliga).", "\n\nFirst: a restore point (p turns it off).") } else { "" };
                let text = format!("{} {} tweak(s)?\n\n{}{rp}", tr("Aplicar", "Apply"), ids.len(), self.titles(&ids));
                self.ask(text, Job::Apply(ids, self.restore_point));
            }
            Tab::Apps => {
                let names: Vec<_> = ids.iter().filter_map(|id| self.cat.apps.iter().find(|a| &a.id == id)).map(|a| format!("  · {}", a.name)).collect();
                let text = format!("{} {} {}?\n\n{}", tr("Instalar", "Install"), ids.len(), tr("programa(s) pelo winget", "app(s) with winget"), names.join("\n"));
                self.ask(text, Job::Install(ids));
            }
            Tab::Debloat => {
                let ids: Vec<_> = ids.into_iter().filter(|id| self.selectable(id)).collect();
                if ids.is_empty() {
                    self.notice = Some(tr("Não está instalado neste PC", "Not installed on this PC").into());
                    return;
                }
                let names: Vec<_> = ids.iter().filter_map(|id| self.cat.bloat.iter().find(|b| &b.id == id)).map(|b| format!("  · {}", b.name)).collect();
                let text = format!(
                    "{} {} app(s)?\n\n{}\n\n{}",
                    tr("Remover", "Remove"),
                    ids.len(),
                    names.join("\n"),
                    tr("Sai de todos os usuários. Dá para reinstalar pela Microsoft Store.", "Removed for every user. Reinstall from the Microsoft Store.")
                );
                self.ask(text, Job::Debloat(ids));
            }
            Tab::Profiles => {
                let Some(p) = ids.first().and_then(|id| self.cat.profile(id)) else { return };
                let s = self.sys.lock().unwrap().clone();
                let fit: Vec<_> = p.tweaks.iter().filter(|id| self.cat.tweak(id).is_some_and(|t| engine::incompatibility(t, &s).is_none())).cloned().collect();
                let mut text = format!("{} \"{}\"?\n\n{} tweak(s)", tr("Aplicar o perfil", "Apply the profile"), p.title.get(), fit.len());
                if fit.len() < p.tweaks.len() {
                    text += &format!(" ({} {})", p.tweaks.len() - fit.len(), tr("não servem para este PC e ficam de fora", "do not fit this PC and are skipped"));
                }
                if !p.debloat.is_empty() {
                    text += &format!(" + {} app(s) {}", p.debloat.len(), tr("removidos (se instalados)", "removed (if installed)"));
                }
                if self.restore_point {
                    text += tr("\nAntes: ponto de restauração.", "\nFirst: a restore point.");
                }
                self.ask(text, Job::Profile(p.id.clone(), self.restore_point));
            }
            Tab::Repairs => {
                let Some(r) = ids.first().and_then(|id| repair::find(id)) else { return };
                self.ask(format!("{}?\n\n{}", r.title(), r.description()), Job::Repair(r.id.to_string()));
            }
            Tab::Log => {}
        }
    }

    fn undo(&mut self) {
        match self.tab {
            Tab::Tweaks => {
                let ids: Vec<_> = self.targets().into_iter().filter(|id| self.status.get(id).is_some_and(|s| s.revertible)).collect();
                if ids.is_empty() {
                    self.notice = Some(tr("Nada para desfazer: não foi aplicado pelo ntoskrnl", "Nothing to undo: not applied by ntoskrnl").into());
                    return;
                }
                let text = format!("{} {} tweak(s)?\n\n{}", tr("Desfazer", "Undo"), ids.len(), self.titles(&ids));
                self.ask(text, Job::Revert(ids));
            }
            Tab::Profiles => {
                if let Some(p) = self.current().and_then(|id| self.cat.profile(&id)) {
                    let text = format!("{} \"{}\"?", tr("Desfazer os tweaks do perfil", "Undo the tweaks of profile"), p.title.get());
                    self.ask(text, Job::ProfileRevert(p.id.clone()));
                }
            }
            _ => {}
        }
    }

    fn switch(&mut self, step: usize) {
        let i = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.goto(TABS[(i + step) % TABS.len()]);
    }

    fn goto(&mut self, t: Tab) {
        if self.tab != t {
            self.tab = t;
            self.filter.clear();
            self.filtering = false;
            self.category = None;
        }
    }

    fn move_by(&mut self, d: i64, n: usize) {
        if self.tab == Tab::Log {
            // no log, para cima = mais antigo (log_scroll conta linhas a partir do fim)
            let max = self.log.len() as i64;
            self.log_scroll = (self.log_scroll as i64 - d).clamp(0, max) as usize;
            return;
        }
        if n == 0 {
            return;
        }
        let l = self.list_mut();
        l.cursor = (l.cursor as i64 + d).clamp(0, n as i64 - 1) as usize;
    }
}

fn summary(r: &[engine::Outcome]) -> String {
    let bad = r.iter().filter(|x| x.1.is_err()).count();
    if bad == 0 {
        format!("{} ok", r.len())
    } else {
        format!("{} ok, {bad} {}", r.len() - bad, tr("com falha (veja o Log)", "failed (see the Log)"))
    }
}

/// Roda um trabalho fora da thread da tela. Devolve o resumo que aparece na barra de status.
fn work(job: Job, cat: &Catalog, st: &Mutex<State>, sys_info: &Mutex<SysInfo>, tx: &Sender<Msg>, log: &mut dyn FnMut(String)) -> String {
    let send = |m: Msg| {
        let _ = tx.send(m);
    };
    let refresh = |st: &State| send(Msg::Status(engine::status(cat, st, &sys_info.lock().unwrap())));
    let restore_point = |log: &mut dyn FnMut(String)| {
        log(tr("Criando ponto de restauração...", "Creating restore point...").into());
        match engine::restore_point() {
            Ok(()) => log("  ok".into()),
            Err(e) => log(format!("  {}: {e}", tr("não deu (seguindo mesmo assim)", "failed (continuing anyway)"))),
        }
    };

    match job {
        Job::Init => {
            let info = sys::sysinfo();
            log(format!("{}: {}", tr("Sistema", "System"), info.os_name()));
            log(format!("CPU: {} · GPU: {} · RAM: {} GB", info.cpu, info.gpus, info.ram_gb));
            *sys_info.lock().unwrap() = info.clone();
            send(Msg::Info(info));
            let state = st.lock().unwrap();
            refresh(&state);
            send(Msg::Bloat(apps::installed_bloat()));
            if !apps::winget_available() {
                log(tr("winget não encontrado: instale o \"Instalador de Aplicativo\" pela Microsoft Store", "winget not found: install \"App Installer\" from the Microsoft Store").into());
            }
            send(Msg::Apps(apps::installed_apps()));
            send(Msg::Drift(engine::drift(cat, &state)));
            String::new()
        }
        Job::Apply(ids, rp) => {
            if rp {
                restore_point(log);
            }
            let mut state = st.lock().unwrap();
            let r = engine::apply(cat, &mut state, &sys_info.lock().unwrap().clone(), &ids, log);
            refresh(&state);
            send(Msg::Drift(engine::drift(cat, &state)));
            summary(&r)
        }
        Job::Revert(ids) => {
            let mut state = st.lock().unwrap();
            let r = engine::revert(cat, &mut state, &ids, log);
            refresh(&state);
            summary(&r)
        }
        Job::Profile(id, rp) => {
            let Some(p) = cat.profile(&id) else { return String::new() };
            let s = sys_info.lock().unwrap().clone();
            if rp {
                restore_point(log);
            }
            let ids: Vec<_> = p.tweaks.iter().filter(|id| cat.tweak(id).is_some_and(|t| engine::incompatibility(t, &s).is_none())).cloned().collect();
            let mut state = st.lock().unwrap();
            let mut r = engine::apply(cat, &mut state, &s, &ids, log);
            refresh(&state);
            if !p.debloat.is_empty() {
                let present = apps::installed_bloat();
                let list: Vec<_> = cat.bloat.iter().filter(|b| p.debloat.contains(&b.id) && present.contains(&b.id)).collect();
                r.extend(apps::remove_bloat(&list, log));
                send(Msg::Bloat(apps::installed_bloat()));
            }
            summary(&r)
        }
        Job::ProfileRevert(id) => {
            let Some(p) = cat.profile(&id) else { return String::new() };
            let mut state = st.lock().unwrap();
            let ids: Vec<_> = p.tweaks.iter().filter(|id| state.backups.contains_key(*id)).cloned().collect();
            let r = engine::revert(cat, &mut state, &ids, log);
            refresh(&state);
            summary(&r)
        }
        Job::Install(ids) => {
            let list: Vec<_> = cat.apps.iter().filter(|a| ids.contains(&a.id)).collect();
            let r = apps::install(&list, log);
            send(Msg::Apps(apps::installed_apps()));
            summary(&r)
        }
        Job::Upgrade => {
            let r = apps::upgrade_all(log);
            send(Msg::Apps(apps::installed_apps()));
            summary(&r)
        }
        Job::Debloat(ids) => {
            let list: Vec<_> = cat.bloat.iter().filter(|b| ids.contains(&b.id)).collect();
            let r = apps::remove_bloat(&list, log);
            send(Msg::Bloat(apps::installed_bloat()));
            summary(&r)
        }
        Job::Repair(id) => {
            let r = repair::run(&id, log);
            let restart = repair::find(&id).is_some_and(|r| r.restart);
            match r {
                Ok(()) => {
                    log("  ok".into());
                    format!("ok{}", if restart { tr(" (reinicie o PC)", " (restart the PC)") } else { "" })
                }
                Err(e) => {
                    log(format!("  {}: {e}", tr("falhou", "failed")));
                    format!("{}: {e}", tr("falhou", "failed"))
                }
            }
        }
        Job::Check => {
            let state = st.lock().unwrap();
            let d = engine::drift(cat, &state);
            let msg = if d.is_empty() {
                tr("Tudo que o ntoskrnl aplicou continua valendo", "Everything ntoskrnl applied is still in place").to_string()
            } else {
                String::new()
            };
            send(Msg::Drift(d));
            msg
        }
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Desenha cada aba com dados simulados (NTOSKRNL_SHOT=pasta grava os quadros em texto para conferir)
    #[test]
    fn draws_every_tab() {
        std::env::set_var("NTOSKRNL_DATA", std::env::temp_dir().join("ntoskrnl-test"));
        let mut app = App::new(Catalog::load(None).unwrap(), true);
        app.start(Job::Init, "init");
        while app.busy.is_some() {
            if let Ok(m) = app.rx.recv_timeout(Duration::from_secs(10)) {
                app.handle(m);
            }
        }
        let mut term = Terminal::new(TestBackend::new(150, 40)).unwrap();
        for (i, tab) in TABS.iter().enumerate() {
            app.tab = *tab;
            app.lists[i].cursor = 1;
            if *tab == Tab::Tweaks {
                app.key(KeyEvent::from(KeyCode::Char(' ')));
                app.key(KeyEvent::from(KeyCode::Enter));
            }
            term.draw(|f| ui::draw(f, &mut app)).unwrap();
            app.confirm = None;
            if let Some(dir) = std::env::var_os("NTOSKRNL_SHOT") {
                let buf = term.backend().buffer();
                let text: String = (0..buf.area.height)
                    .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect::<String>() + "\n")
                    .collect();
                std::fs::write(std::path::Path::new(&dir).join(format!("{}.txt", i + 1)), text).unwrap();
            }
        }
    }
}
