// Pas de console en plus de la fenêtre sous Windows (release uniquement)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    twitch_desk_lib::run()
}
