//! Import par lot d'anciens giveaways (page Alertes, giveaways passés), au format JSON : une liste
//! d'objets (ou `{ "giveaways": [...] }`). Chaque giveaway est contrôlé à part ; « Vérifier »
//! (`dry_run`) ne fait que le contrôle, l'import enregistre ceux qui sont valides, déjà terminés,
//! comme « Ajouter un ancien giveaway ».

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::app::App;
use crate::db::{GiveawayDetails, GiveawayInput};

/// Giveaways d'un import, au plus
const MAX_ITEMS: usize = 500;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WinnerItem {
    Login(String),
    Full { login: String, chance: Option<Value> },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum GiftRef {
    Id(i64),
    Title(String),
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Conditions {
    follow: Option<bool>,
    follow_days: Option<i64>,
    /// Ancienneté écrite comme dans le formulaire (« 7j », « 2h 30min ») : prime sur followDays
    follow_duration: Option<String>,
    presence: Option<bool>,
    discord: Option<bool>,
    greeting: Option<bool>,
    chat_minutes: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImportItem {
    channel: String,
    date: String,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    draw_at: Option<String>,
    #[serde(default)]
    winners: Vec<WinnerItem>,
    #[serde(default)]
    gifts: Vec<GiftRef>,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    ticket_unique: Option<bool>,
    #[serde(default)]
    ticket_max: Option<i64>,
    #[serde(default)]
    reward_name: Option<String>,
    #[serde(default)]
    reward_cost: Option<i64>,
    #[serde(default)]
    conditions: Conditions,
}

/// Résultat d'un giveaway de l'import (numéro à partir de 1)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLine {
    index: usize,
    channel: String,
    date: String,
    ok: bool,
    /// Erreur (non importé) ou précisions (cadeaux inconnus ignorés…)
    message: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// Giveaways valides (vérification) ou importés
    ok: usize,
    errors: usize,
    imported: bool,
    lines: Vec<ImportLine>,
}

/// Date d'un giveaway : RFC 3339, sinon « AAAA-MM-JJ HH:MM » ou « AAAA-MM-JJ » en heure locale
/// (un jour seul : midi) ; dans le passé
fn parse_date(text: &str) -> Result<String, String> {
    use chrono::TimeZone;
    let text = text.trim();
    let utc = if let Ok(d) = chrono::DateTime::parse_from_rfc3339(text) {
        d.with_timezone(&chrono::Utc)
    } else {
        let naive = chrono::NaiveDateTime::parse_from_str(&text.replace('T', " "), "%Y-%m-%d %H:%M")
            .or_else(|_| chrono::NaiveDateTime::parse_from_str(&text.replace('T', " "), "%Y-%m-%d %H:%M:%S"))
            .or_else(|_| chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|d| d.and_hms_opt(12, 0, 0).expect("midi")))
            .map_err(|_| format!("date illisible « {text} » (ex. 2026-10-20 ou 2026-10-20 21:30)"))?;
        chrono::Local.from_local_datetime(&naive).earliest().ok_or_else(|| format!("date inexistante « {text} »"))?.with_timezone(&chrono::Utc)
    };
    if utc > chrono::Utc::now() + chrono::Duration::minutes(5) {
        return Err("date dans le futur".into());
    }
    Ok(utc.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// Giveaway prêt à enregistrer : fiche validée, date, gagnants ; précisions éventuelles
struct Checked {
    input: GiveawayInput,
    at: String,
    winners: Vec<(String, Option<String>)>,
    notes: Vec<String>,
}

fn check(item: ImportItem, gifts: &[crate::db::Gift]) -> Result<Checked, String> {
    let at = parse_date(&item.date)?;
    let draw_at = match item.draw_at.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(d) => Some(parse_date(d).map_err(|e| format!("tirage : {e}"))?),
        None => None,
    };
    let mut notes = vec![];
    let mut gift_ids = vec![];
    for g in &item.gifts {
        let found = match g {
            GiftRef::Id(id) => gifts.iter().find(|x| x.id == *id),
            GiftRef::Title(t) => gifts.iter().find(|x| x.title.trim().eq_ignore_ascii_case(t.trim())),
        };
        match found {
            Some(x) if !gift_ids.contains(&x.id) => gift_ids.push(x.id),
            Some(_) => {}
            None => notes.push(match g {
                GiftRef::Id(id) => format!("cadeau n° {id} inconnu, ignoré"),
                GiftRef::Title(t) => format!("cadeau « {t} » absent du catalogue, ignoré"),
            }),
        }
    }
    let c = &item.conditions;
    let input = GiveawayInput {
        channel: item.channel,
        kind: item.kind.unwrap_or_else(|| "command".into()).trim().to_lowercase(),
        command: item.command,
        url: item.url,
        gift_ids,
        details: GiveawayDetails {
            ticket_unique: item.ticket_unique,
            ticket_max: item.ticket_max,
            reward_name: item.reward_name,
            reward_cost: item.reward_cost,
            draw_at,
            require_follow: c.follow,
            require_presence: c.presence,
            require_discord: c.discord,
            notes: item.notes,
            require_greeting: c.greeting,
            min_chat_minutes: c.chat_minutes,
            follow_min_secs: match c.follow_duration.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
                Some(d) => Some(crate::giveaways::parse_duration(d).ok_or_else(|| format!("followDuration « {d} » invalide (ex. 7j, 2h 30min)"))?),
                None => c.follow_days.and_then(|d| d.checked_mul(86_400)),
            },
        },
    };
    let input = crate::giveaways::validate_input(input)?;
    let mut winners: Vec<(String, Option<String>)> = vec![];
    for w in item.winners {
        let (login, chance) = match w {
            WinnerItem::Login(l) => (l, None),
            WinnerItem::Full { login, chance } => (login, chance),
        };
        let login = crate::commands::normalize_login(&login).ok_or_else(|| format!("gagnant invalide « {} »", login.trim()))?;
        let chance = match chance {
            None | Some(Value::Null) => None,
            Some(Value::Number(n)) => Some(crate::giveaways::bots::normalize_chance(&n.to_string()).ok_or("chance invalide")?),
            Some(Value::String(s)) if s.trim().is_empty() => None,
            Some(Value::String(s)) => Some(crate::giveaways::bots::normalize_chance(&s).ok_or_else(|| format!("chance invalide « {s} »"))?),
            Some(_) => return Err("chance invalide".into()),
        };
        if !winners.iter().any(|(l, _)| *l == login) {
            winners.push((login, chance));
        }
    }
    Ok(Checked { input, at, winners, notes })
}

/// Liste des giveaways du JSON (liste, ou objet `giveaways`)
fn items_of(json: &str) -> Result<Vec<Value>, String> {
    let value: Value = serde_json::from_str(json).map_err(|e| format!("JSON invalide : {e}"))?;
    let list = match value {
        Value::Array(list) => list,
        Value::Object(mut map) => match map.remove("giveaways") {
            Some(Value::Array(list)) => list,
            _ => return Err("JSON attendu : une liste de giveaways, ou un objet avec une liste « giveaways »".into()),
        },
        _ => return Err("JSON attendu : une liste de giveaways".into()),
    };
    if list.is_empty() {
        return Err("Aucun giveaway dans le JSON".into());
    }
    if list.len() > MAX_ITEMS {
        return Err(format!("{MAX_ITEMS} giveaways au plus par import"));
    }
    Ok(list)
}

/// Contrôle (`dry_run`) ou import des giveaways du JSON ; un giveaway déjà enregistré (même
/// chaîne, même date) n'est pas importé deux fois
#[tauri::command]
pub async fn import_past_giveaways(app: State<'_, Arc<App>>, json: String, dry_run: bool) -> Result<ImportReport, String> {
    let app: Arc<App> = app.inner().clone();
    tauri::async_runtime::spawn_blocking(move || import(&app, &json, dry_run)).await.map_err(|e| e.to_string())?
}

fn import(app: &Arc<App>, json: &str, dry_run: bool) -> Result<ImportReport, String> {
    let items = items_of(json)?;
    let gifts = app.db.gifts().map_err(|e| e.to_string())?;
    let mut lines = vec![];
    let mut seen: Vec<String> = vec![];
    let mut imported_any = false;
    for (i, raw) in items.into_iter().enumerate() {
        let channel_hint = raw.get("channel").and_then(Value::as_str).unwrap_or("").to_owned();
        let date_hint = raw.get("date").and_then(Value::as_str).unwrap_or("").to_owned();
        let mut line = ImportLine { index: i + 1, channel: channel_hint, date: date_hint, ok: false, message: None };
        let checked = serde_json::from_value::<ImportItem>(raw)
            .map_err(|e| format!("champ manquant ou mal écrit : {e}"))
            .and_then(|item| check(item, &gifts));
        match checked {
            Err(e) => line.message = Some(e),
            Ok(c) => {
                line.channel = c.input.channel.clone();
                line.date = c.at.clone();
                let winner = c.winners.first().map(|(w, _)| w.as_str());
                let key = format!(
                    "{}|{}|{}|{}|{}|{}",
                    c.input.channel,
                    c.at,
                    c.input.kind,
                    c.input.command.as_deref().unwrap_or("").to_lowercase(),
                    c.input.url.as_deref().unwrap_or("").to_lowercase(),
                    winner.unwrap_or("").to_lowercase()
                );
                let exists = match app.db.giveaway_at(&c.input.channel, &c.at, &c.input.kind, c.input.command.as_deref(), c.input.url.as_deref(), winner) {
                    Ok(found) => seen.contains(&key) || found,
                    Err(e) => {
                        line.message = Some(format!("lecture impossible : {e}"));
                        lines.push(line);
                        continue;
                    }
                };
                if exists {
                    line.message = Some("déjà enregistré (même chaîne, même date, même giveaway)".into());
                } else {
                    seen.push(key);
                    // Erreur d'enregistrement : ligne en erreur dans le rapport, l'import continue
                    match if dry_run { Ok(0) } else { crate::commands::insert_past_giveaway(app, &c.input, &c.at, &c.winners) } {
                        Ok(_) => {
                            line.ok = true;
                            imported_any |= !dry_run;
                            if !c.notes.is_empty() {
                                line.message = Some(c.notes.join(" ; "));
                            }
                        }
                        Err(e) => line.message = Some(format!("non enregistré : {e}")),
                    }
                }
            }
        }
        lines.push(line);
    }
    if imported_any {
        app.giveaways.send(crate::giveaways::MonitorCmd::Reload);
        app.sync.poke();
    }
    let ok = lines.iter().filter(|l| l.ok).count();
    Ok(ImportReport { ok, errors: lines.len() - ok, imported: !dry_run, lines })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gift(id: i64, title: &str) -> crate::db::Gift {
        serde_json::from_value(serde_json::json!({ "id": id, "title": title, "image": null, "groupId": null, "keywords": [] })).unwrap()
    }

    #[test]
    fn items_and_checks() {
        assert!(items_of("[]").is_err());
        assert!(items_of("{\"x\":1}").is_err());
        assert_eq!(items_of(r#"{"giveaways":[{"channel":"a","date":"2026-01-01"}]}"#).unwrap().len(), 1);
        let gifts = vec![gift(1, "Voiles obsidiennes")];
        let item: ImportItem = serde_json::from_value(serde_json::json!({
            "channel": "SaEnz203", "date": "2026-10-20 21:30", "type": "streamelements",
            "url": "https://StreamElements.com/saenz203/giveaway/6ae9", "gifts": ["voiles obsidiennes", "Inconnu", 1],
            "winners": ["bilge_rat", { "login": "@Bob", "chance": "20,92 %" }, { "login": "eve", "chance": 3 }],
            "conditions": { "presence": true, "chatMinutes": 30 }
        }))
        .unwrap();
        let c = check(item, &gifts).unwrap();
        assert_eq!(c.input.channel, "saenz203");
        assert_eq!(c.input.url.as_deref(), Some("https://streamelements.com/saenz203/giveaway/6ae9"));
        assert_eq!(c.input.gift_ids, vec![1]);
        assert_eq!(c.notes.len(), 1);
        assert_eq!(c.winners, vec![("bilge_rat".into(), None), ("bob".into(), Some("20.92%".into())), ("eve".into(), Some("3%".into()))]);
        assert_eq!(c.input.details.require_presence, Some(true));
        // Date : jour seul (midi local), RFC 3339, illisible, future
        assert!(parse_date("2026-10-20").is_ok());
        assert_eq!(parse_date("2026-10-20T19:30:00Z").unwrap(), "2026-10-20T19:30:00Z");
        assert!(parse_date("20/10/2026").is_err());
        assert!(parse_date("2999-01-01").is_err());
        let bad: ImportItem = serde_json::from_value(serde_json::json!({ "channel": "x y", "date": "2026-01-01" })).unwrap();
        assert!(check(bad, &gifts).is_err());
    }
}
