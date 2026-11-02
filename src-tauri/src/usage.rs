//! Ressources utilisées par l'application : processus principal et tous ses processus enfants
//! (WebView2 de la fenêtre principale et de la fenêtre des streams), relevées toutes les 2 s et
//! envoyées à l'interface (événement « app-usage »)
//!
//! Trafic : Windows ne donne le trafic réseau d'un programme qu'avec les droits administrateur.
//! On additionne donc les octets lus et écrits par ces processus (compteurs d'entrées-sorties) :
//! le processus réseau de WebView2 y compte la vidéo des streams. C'est une estimation, qui
//! inclut un peu d'accès disque (cache, base) ; ailleurs que sous Windows, ces compteurs ne
//! portent que sur le disque, le trafic n'est donc pas donné.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter};

const EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppUsage {
    /// Part du processeur de l'ordinateur (0 à 100, comme le Gestionnaire des tâches)
    pub cpu: f32,
    /// Mémoire utilisée, en octets
    pub memory: u64,
    /// Nombre de processus comptés (application et WebView2)
    pub processes: usize,
    /// Trafic estimé, en octets par seconde (None : pas mesurable sur ce système)
    pub down: Option<u64>,
    pub up: Option<u64>,
}

/// Le processus `root` et tous ses descendants
fn family(sys: &System, root: Pid) -> HashSet<Pid> {
    let mut set = HashSet::from([root]);
    // Les parents peuvent apparaître après leurs enfants : on recommence tant que l'ensemble grandit
    loop {
        let before = set.len();
        for (pid, p) in sys.processes() {
            if p.parent().is_some_and(|parent| set.contains(&parent)) {
                set.insert(*pid);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

fn measure(sys: &System, root: Pid, cores: f32, elapsed: f32) -> AppUsage {
    let pids = family(sys, root);
    let (mut cpu, mut memory, mut read, mut written) = (0f32, 0u64, 0u64, 0u64);
    for pid in &pids {
        if let Some(p) = sys.process(*pid) {
            cpu += p.cpu_usage();
            memory += p.memory();
            let io = p.disk_usage();
            read += io.read_bytes;
            written += io.written_bytes;
        }
    }
    let per_second = |bytes: u64| (bytes as f32 / elapsed.max(0.1)) as u64;
    let traffic = cfg!(windows);
    AppUsage {
        cpu: (cpu / cores).clamp(0.0, 100.0),
        memory,
        processes: pids.len(),
        down: traffic.then(|| per_second(read)),
        up: traffic.then(|| per_second(written)),
    }
}

pub fn spawn(app: AppHandle) {
    let Ok(root) = sysinfo::get_current_pid() else {
        tracing::warn!("ressources de l'application : processus courant introuvable");
        return;
    };
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
    // Relevé dans un fil à part : parcourir les processus prend quelques millisecondes
    std::thread::spawn(move || {
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_disk_usage();
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let mut last = Instant::now();
        loop {
            std::thread::sleep(EVERY);
            sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
            let now = Instant::now();
            let usage = measure(&sys, root, cores, now.duration_since(last).as_secs_f32());
            last = now;
            if app.emit("app-usage", &usage).is_err() {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_current_process() {
        let root = sysinfo::get_current_pid().unwrap();
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_disk_usage();
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        let usage = measure(&sys, root, 1.0, 2.0);
        assert!(usage.processes >= 1);
        assert!(usage.memory > 0);
        assert!((0.0..=100.0).contains(&usage.cpu));
    }
}
