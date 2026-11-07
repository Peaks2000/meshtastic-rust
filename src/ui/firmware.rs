use super::*;
use crate::firmware::Firmware;
use std::{path::PathBuf, sync::mpsc};

pub struct Picker {
    pub file: RefCell<Option<PathBuf>>,
    pub source: RefCell<Option<Firmware>>,
    label: gtk::Label,
    download: gtk::Button,
    lookup: gtk::Button,
    board: RefCell<Option<String>>,
    generation: Cell<u64>,
    busy: Cell<bool>,
    sender: mpsc::Sender<(u64, Result<Firmware, String>)>,
}
impl Picker {
    pub fn build(ui: &Rc<Ui>, options: &gtk::Box) -> Rc<Self> {
        let (sender, receiver) = mpsc::channel();
        let picker = Rc::new(Self {
            file: RefCell::new(None),
            source: RefCell::new(None),
            label: label("Connect a node to find its firmware", "muted"),
            download: button("Download firmware", ""),
            lookup: button("Check for firmware", ""),
            board: RefCell::new(None),
            generation: Cell::new(0),
            busy: Cell::new(false),
            sender,
        });
        picker
            .label
            .set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        picker.download.set_sensitive(false);
        options.append(&picker.label);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        actions.append(&picker.lookup);
        actions.append(&picker.download);
        options.append(&actions);
        let weak = Rc::downgrade(ui);
        let p = Rc::downgrade(&picker);
        picker.lookup.connect_clicked(move |_| {
            if let (Some(ui), Some(p)) = (weak.upgrade(), p.upgrade()) {
                p.refresh(&ui, true);
            }
        });
        let weak = Rc::downgrade(ui);
        let p = Rc::downgrade(&picker);
        picker.download.connect_clicked(move |_| {
            if let (Some(ui), Some(p)) = (weak.upgrade(), p.upgrade()) {
                if ui.flash_busy.get() || p.busy.get() {
                    return;
                }
                let source = p.source.borrow().clone();
                if let Some(source) = source {
                    p.run(source.board, Some(source.tag));
                }
            }
        });
        let p = Rc::downgrade(&picker);
        let weak = Rc::downgrade(ui);
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let (Some(p), Some(ui)) = (p.upgrade(), weak.upgrade()) else {
                return glib::ControlFlow::Break;
            };
            while let Ok((generation, result)) = receiver.try_recv() {
                if generation != p.generation.get() {
                    continue;
                }
                p.busy.set(false);
                p.lookup.set_sensitive(true);
                match result {
                    Ok(source) if Some(source.board.clone()) == p.current_board(&ui) => {
                        if let Some(path) = &source.path {
                            *p.file.borrow_mut() = Some(path.clone());
                            p.label
                                .set_text(&format!("{} · {} · ready", source.board, source.tag));
                        } else {
                            p.label.set_text(&format!(
                                "{} · {} · {} MB",
                                source.board,
                                source.title,
                                source.size / 1_000_000
                            ));
                        }
                        *p.source.borrow_mut() = Some(source);
                        p.download.set_sensitive(true);
                    }
                    Ok(_) => {
                        p.download.set_sensitive(false);
                        p.board.borrow_mut().take();
                    }
                    Err(error) => {
                        p.label.set_text("Firmware lookup failed");
                        p.download.set_sensitive(false);
                        ui.notice(&error);
                    }
                }
            }
            glib::ControlFlow::Continue
        });
        picker
    }
    fn current_board(&self, ui: &Ui) -> Option<String> {
        let state = ui.state.borrow();
        if !state.connected || ui.recovery_picker.active_id().as_deref() != Some("normal") {
            return None;
        }
        state
            .nodes
            .get(&state.own)
            .map(|node| node.hardware.clone())
            .filter(|board| !board.is_empty() && board != "UNSET")
    }
    pub fn refresh(&self, ui: &Ui, force: bool) {
        if ui.flash_busy.get() || (self.busy.get() && !force) {
            return;
        }
        let board = self.current_board(ui);
        if !force && *self.board.borrow() == board {
            return;
        }
        if let Some(board) = board {
            self.run(board, None);
        } else if force {
            ui.notice(
                "Connect to a node to identify its board, or choose a firmware file for recovery.",
            );
        }
    }
    fn run(&self, board: String, tag: Option<String>) {
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();
        *self.board.borrow_mut() = Some(board.clone());
        self.busy.set(true);
        self.download.set_sensitive(false);
        self.lookup.set_sensitive(false);
        self.label.set_text(if tag.is_some() {
            "Downloading and verifying firmware…"
        } else {
            "Finding firmware…"
        });
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = crate::firmware::fetch(&board, tag.as_deref())
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send((generation, result));
        });
    }
    pub fn choose(&self, path: PathBuf) {
        self.generation.set(self.generation.get() + 1);
        self.busy.set(false);
        self.lookup.set_sensitive(true);
        self.download.set_sensitive(false);
        self.source.borrow_mut().take();
        self.label.set_text(&path.display().to_string());
        *self.file.borrow_mut() = Some(path);
    }
    pub fn target(&self, path: &std::path::Path, board: &str) -> Option<String> {
        self.source
            .borrow()
            .as_ref()
            .filter(|s| s.path.as_deref() == Some(path) && s.board == board)
            .map(|s| s.target.clone())
    }
}
