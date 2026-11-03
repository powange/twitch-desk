//! Commandes de l'application déclarées à Tauri : sans cette liste, toutes les commandes
//! seraient permises à toute fenêtre servie par l'application, y compris la fenêtre des
//! streams (qui charge un script de Twitch). Elles ne sont accordées qu'à la fenêtre
//! principale (capabilities/default.json). La liste doit suivre `generate_handler!` (lib.rs) :
//! vérifiée à la compilation.

const COMMANDS: &[&str] = &[
    "get_snapshot",
    "start_login",
    "cancel_login",
    "remove_account",
    "restart_account",
    "set_points_enabled",
    "add_channel",
    "remove_channel",
    "reorder_channels",
    "set_channel_options",
    "set_paused",
    "get_settings",
    "save_settings",
    "get_paths",
    "connect_streamelements",
    "set_streamelements_token",
    "disconnect_streamelements",
    "buy_se_tickets",
    "set_se_refresh",
    "set_max_balance",
    "watch_stream",
    "import_sound",
    "read_sound",
    "list_logs",
    "read_log",
    "export_log",
    "set_drops_enabled",
    "add_drop_game",
    "remove_drop_game",
    "set_drop_game_enabled",
    "reorder_drop_games",
    "refresh_drops",
    "add_giveaway_channel",
    "import_giveaway_channels",
    "remove_giveaway_channel",
    "set_ignored_commands",
    "save_window_state",
    "participate_giveaway",
    "dismiss_giveaway",
    "close_giveaway",
    "delete_giveaway",
    "dismiss_mention",
    "create_giveaway",
    "inspect_giveaway_channel",
    "get_gift_catalog",
    "save_gift",
    "set_gift_favorite",
    "follow_status",
    "greeting_status",
    "say_hello",
    "test_winner_phrase",
    "get_detection_rules",
    "set_detection_rules",
    "get_chat_log",
    "set_giveaway_command",
    "get_chat_log_meta",
    "set_manual_winner",
    "clear_giveaway_winner",
    "streamer_discord",
    "follow_channel",
    "delete_gift",
    "save_gift_group",
    "delete_gift_group",
    "get_wins",
    "set_prize",
    "resolve_category",
    "search_categories",
    "channel_rewards",
    "reopen_giveaway",
    "cancel_win_reply",
    "set_giveaway_winners",
    "se_giveaway_info",
    "past_giveaways",
    "refresh_channels",
    "refresh_giveaways",
    "create_past_giveaway",
    "import_past_giveaways",
    "set_phone_actions",
    "game_box_arts",
    "save_streamer",
    "get_streamers",
    "scan_streams",
    "scanner_ignore",
    "scanner_refilter",
    "get_sync_config",
    "save_sync_config",
    "test_sync",
    "sync_now",
    "set_sync_skipped",
    "sync_resync",
    "save_sync_instance",
    "sync_pair_start",
    "sync_devices",
    "sync_revoke_device",
    "export_gifts",
    "import_gifts",
    "update_giveaway",
    "add_giveaway_game",
    "remove_giveaway_game",
    "take_start_page",
];

fn main() {
    check_commands();
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)))
        .expect("tauri-build");
}

/// Compare la liste ci-dessus à celle de `generate_handler!` dans src/lib.rs
fn check_commands() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    let lib = std::fs::read_to_string("src/lib.rs").expect("src/lib.rs");
    let start = lib.find("generate_handler![").expect("generate_handler! dans lib.rs") + "generate_handler![".len();
    let end = start + lib[start..].find(']').expect("fin de generate_handler!");
    let mut handled: Vec<&str> = lib[start..end]
        .split(',')
        .map(|c| c.trim().rsplit("::").next().unwrap_or("").trim())
        .filter(|c| !c.is_empty())
        .collect();
    let mut declared = COMMANDS.to_vec();
    handled.sort_unstable();
    declared.sort_unstable();
    assert_eq!(handled, declared, "build.rs : COMMANDS doit reprendre exactement les commandes de generate_handler! (lib.rs)");
}
