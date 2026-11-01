# Twitch Desk

Application de bureau (Windows, Linux) pour Twitch, sur plusieurs comptes à la fois.

Version 0.1 : **points de chaîne**. L'application simule le visionnage des chaînes choisies sans ouvrir de lecteur ni télécharger la vidéo, et récupère le bonus automatiquement. Les drops et le suivi des giveaways arrivent dans les versions suivantes.

Stack : Tauri 2, cœur en Rust (tokio, reqwest, SQLite), interface Vue 3 + Nuxt UI.

## Développer

Prérequis : Rust stable, Node 24. Sous Linux, les paquets WebKitGTK de Tauri (`libwebkit2gtk-4.1-dev`, `libappindicator3-dev`, `librsvg2-dev`).

```bash
npm install
npm run tauri dev       # application avec rechargement à chaud
cargo test --manifest-path src-tauri/Cargo.toml
npm run typecheck
```

## Comment ça marche

- **Connexion** : code d'appareil (twitch.tv/activate) avec l'identifiant client de l'application Android TV de Twitch. Ses jetons passent sans contrôle d'intégrité sur l'API interne de Twitch. Les jetons sont rangés dans le Gestionnaire d'identification de Windows (Secret Service sous Linux, fichier `secrets.json` en mode 600 si aucun trousseau n'est disponible).
- **Visionnage** : toutes les 10 s, liste de lecture HLS en qualité minimale et requête `HEAD` sur chaque nouveau segment ; toutes les 59 s, événement `minute-watched`. Twitch ne compte que 2 chaînes à la fois par compte : l'application prend les 2 premières chaînes en live de la liste.
- **Événements en direct** : PubSub (`pubsub-edge.twitch.tv`) pour le bonus disponible, les gains, le début et la fin des lives. Le solde est aussi relu régulièrement, ce qui récupère le bonus si PubSub est coupé.
- **Requêtes internes (GQL)** : leurs empreintes sont dans [`operations.json`](operations.json). L'application embarque ce fichier et relit la version de la branche `main` au démarrage : quand Twitch change une empreinte, il suffit de corriger ce fichier, sans publier de nouvelle version. Chaque opération accepte plusieurs empreintes, essayées dans l'ordre.

Ces mécanismes reprennent les principes de [TwitchDropsMiner](https://github.com/DevilXD/TwitchDropsMiner), [Twitch-Channel-Points-Miner-v2](https://github.com/rdavydov/Twitch-Channel-Points-Miner-v2) et [twitch-drops-miner](https://github.com/ohne-b/twitch-drops-miner). Aucun code n'en est copié.

Utiliser ces outils va contre l'esprit des conditions d'utilisation de Twitch : un compte peut être sanctionné.

## Organisation

- `src-tauri/src/twitch/` : connexion (`auth.rs`), requêtes GQL (`gql.rs`), PubSub (`pubsub.rs`), visionnage simulé (`watch.rs`).
- `src-tauri/src/miner.rs` : superviseur d'un compte (choix des chaînes, bonus, événements). Un superviseur isolé par compte.
- `src-tauri/src/app.rs` : état partagé, envoyé à l'interface par l'événement `snapshot`.
- `src-tauri/src/db.rs` : base SQLite (comptes, chaînes, gains, réglages).
- `src/` : interface Vue (pages Tableau de bord, Points de chaîne, Comptes, Paramètres).

Données : `%APPDATA%\com.powange.twitchdesk\` sous Windows (`~/.local/share/com.powange.twitchdesk/` sous Linux), avec les journaux dans `logs/` (7 jours).

## Publier une version

1. Monter la version dans `package.json`, `src-tauri/Cargo.toml` et `src-tauri/tauri.conf.json`.
2. Pousser un tag : `git tag v0.2.0 && git push origin v0.2.0`.
3. Le workflow `Release` compile Windows et Linux, signe les paquets et publie la release avec `latest.json`. Les applications installées la proposent au démarrage ou dans les 6 heures.

La signature utilise une clé privée rangée dans les secrets GitHub `TAURI_SIGNING_PRIVATE_KEY` et `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (copie locale : `~/.tauri/twitch-desk.key`). **Si cette clé est perdue, les applications installées ne pourront plus se mettre à jour** : garde-en une sauvegarde hors de cette machine.
