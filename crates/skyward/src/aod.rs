//! Carries out the panel's own doze that `skyward_ui::aod` decides on.
//!
//! Every write here is best-effort. A panel whose driver refuses one is put
//! out the ordinary way for the rest of the run, and why goes to the kernel
//! log once.

use skyward_config::Aod;
use skyward_ui::aod::{Doze, Driver, Feature, Plan, Shown, Want};
use std::fs;
use std::path::PathBuf;

/// The panel's doze as this run of the recovery has it.
pub struct Panel {
    doze: Doze,
    dir: PathBuf,
    node: PathBuf,
    driver: Option<Driver>,
    shown: Shown,
    /// Set once the driver has refused, so the recovery stops asking.
    given_up: bool,
}

impl Panel {
    pub fn new(aod: &Aod) -> Self {
        let feature = |f: &skyward_config::Feature| Feature { id: f.id, value: f.value.clone() };
        let doze = Doze {
            enter: aod.enter.iter().map(feature).collect(),
            enter_cmds: aod.enter_cmds.clone(),
            leave: aod.leave.iter().map(feature).collect(),
            leave_cmds: aod.leave_cmds.clone(),
        };
        let (dir, node) = (PathBuf::from(&aod.dir), PathBuf::from(&aod.node));
        let driver = read_driver(&dir, &node);
        Panel { doze, dir, node, driver, shown: Shown::Lit, given_up: false }
    }

    /// Whether a dark panel is to show the face rather than be blacked out.
    pub fn usable(&self) -> bool {
        !self.given_up
    }

    /// Tell the panel what the pages want of it, and give what it is doing
    /// once told.
    pub fn want(&mut self, want: Want) -> Shown {
        let decision = self.doze.plan(self.shown, want, self.driver.as_ref());
        if let Some(refusal) = decision.refusal {
            self.give_up(refusal.why());
        }
        self.shown = decision.shown;
        let (writes, cmds) = match decision.plan {
            Plan::Ordinary => return self.shown,
            Plan::Doze { writes, cmds } | Plan::Wake { writes, cmds } => (writes, cmds),
        };
        for (name, value) in writes {
            let path = self.dir.join(&name);
            if let Err(e) = fs::write(&path, &value) {
                let _ = skyward_sys::kmsg(&format!("the panel's {name} is not set to {value}: {e}"));
            }
        }
        for cmd in cmds {
            if let Err(e) = fs::write(&self.node, cmd.to_string()) {
                self.give_up(&format!("the panel did not take command set {cmd}, so it is put out the ordinary way: {e}"));
                // A panel that did not go into its doze is blacked out by
                // the caller; one that did not come out is lit regardless.
                if self.shown == Shown::Dozing {
                    self.shown = Shown::Blank;
                }
                break;
            }
        }
        self.shown
    }

    fn give_up(&mut self, why: &str) {
        if !self.given_up {
            let _ = skyward_sys::kmsg(why);
        }
        self.given_up = true;
        self.driver = None;
    }
}

/// What the display driver says of itself: the features `disp_param` lists
/// as `name[id]: value`, and whether the node that sends a command set opens
/// for writing. A driver with neither is no driver at all.
fn read_driver(dir: &std::path::Path, node: &std::path::Path) -> Option<Driver> {
    let sending = fs::OpenOptions::new().write(true).open(node).is_ok();
    let features = fs::read_to_string(dir.join("disp_param"))
        .map(|text| {
            text.lines()
                .filter_map(|line| {
                    let (head, _) = line.split_once(':')?;
                    let (name, id) = head.trim().strip_suffix(']')?.split_once('[')?;
                    Some((id.to_string(), name.to_string()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    (sending || !features.is_empty()).then_some(Driver { features, sending })
}
