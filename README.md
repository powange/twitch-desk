# Twitch Desk

Application de bureau (Windows, Linux) pour Twitch, sur plusieurs comptes à la fois.

Ce que fait l'application aujourd'hui :

- **Points de chaîne Twitch** : visionnage simulé des chaînes choisies, sans lecteur ni téléchargement de la vidéo, et récupération automatique du bonus.
- **Points StreamElements** : le compte reste connecté au chat des chaînes choisies, ce qui le place dans la liste des spectateurs où StreamElements distribue ses points, au rythme réglé par chaque streamer. Le solde, la règle de gain et le giveaway en cours sont lus sur l'API publique de StreamElements.
- **Scanner** : lives des catégories surveillées dont le titre contient un mot-clé (mots-clés et catégories à cocher, langues, spectateurs minimum), type de giveaway et cadeaux repérés, colonnes triables, scan automatique toutes les 3 min avec notification.
- **Cadeaux** : catalogue rangé par catégorie et par groupe (image téléversée ou en ligne, catégorie Twitch, groupes par catégorie ou communs à tous les jeux, mots-clés), export/import en JSON, cadeaux suggérés d'après le titre, la description et le message épinglé de la chaîne ; page « Mes gains » avec suivi des lots.
- **Giveaways** : fiche complète (types commande, ticket, StreamElements, points de chaîne ; conditions follow, présence, Discord ; heure du tirage avec rappel), création à la main avec détection d'un giveaway StreamElements actif, catégories surveillées, statut live et clôture automatique en fin de live, surveillance du chat des chaînes choisies, détection d'un giveaway (commande reprise par le chat, lien StreamElements), gagnants annoncés par les bots, mentions de tes pseudos, file d'alertes et notifications Windows, participation en un clic depuis les comptes choisis, participation automatique (désactivée par défaut).
- **Drops** : jeux souhaités par ordre de priorité (même sans campagne en cours), visionnage d'un stream avec les drops activés, réclamation automatique de chaque drop terminé.
- **Tickets StreamElements** : avec le compte StreamElements connecté (fenêtre de connexion de l'app), lecture exacte de nos tickets et achat de tickets depuis l'application.


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
- **Chat** : connexion IRC authentifiée (`irc-ws.chat.twitch.tv`), JOIN des chaînes où la présence est activée, sans jamais envoyer de message. Une connexion anonyme ne suffirait pas : elle n'apparaît pas dans la liste des spectateurs. La présence n'a pas de limite de chaînes.
- **Compte StreamElements** : l'app ouvre `id.streamelements.com/auth/twitch` dans une fenêtre privée et lit le jeton de session dans le cookie `id_token_creator` (ou `se-token`) de `.streamelements.com`, comme le site. Le jeton est vérifié (`kappa/v2/channels/me`, même identifiant Twitch que le compte) puis rangé dans le trousseau. Nos tickets : `GET kappa/v3/giveaways/{chaîne}/{giveaway}/entry` ; achat : `POST` sur la même adresse avec `{"tickets": n}` (en-tête `Authorization: Bearer <jeton>`).
- **StreamElements** : `api.streamelements.com/kappa/v2/channels/{login}` donne l'identifiant de la chaîne, `kappa/v2/points/{id}/{pseudo}` le solde (404 tant que le pseudo n'a rien gagné). Adresses non documentées officiellement. Une baisse du solde (tickets achetés) n'est pas comptée comme un gain.
- **Drops** : le jeton « Android TV » ne voit ni le tableau des campagnes ni leur détail (réponse vide), seulement l'inventaire (campagnes commencées, progression, identifiants de réclamation). Pour un jeu souhaité, l'app cherche dans l'annuaire du jeu les streams avec le filtre `DROPS_ENABLED` ; sans campagne connue, elle fait confirmer par Twitch qu'une campagne est ouverte sur la chaîne (`DropsHighlightService_AvailableDrops`), car un streamer peut garder le tag après une campagne. La chaîne des drops prend une des 2 places de visionnage du compte, avant les points. Progression et fin de drop arrivent par PubSub (`user-drop-events`), réclamation par `DropsPage_ClaimDropRewards`. Après 20 min sans campagne visible, le jeu est mis de côté 1 h.
- **Giveaways** : règles du site Giveaway Twitch (même commande 7 fois sur les 20 derniers messages, 2 au plus par pseudo, réarmement sous 3, 10 min entre deux alertes ; `!ticket` ignoré pendant un giveaway StreamElements). Lecture mutualisée : un seul lecteur par chaîne, la connexion d'un compte déjà présent (StreamElements) sinon une connexion anonyme commune (`justinfanNNNNN`, lecture seule). La participation envoie la commande par une connexion courte du compte (JOIN, message, PART).
- **Requêtes internes (GQL)** : leurs empreintes sont dans [`operations.json`](operations.json). L'application embarque ce fichier et relit la version de la branche `main` au démarrage : quand Twitch change une empreinte, il suffit de corriger ce fichier, sans publier de nouvelle version. Chaque opération accepte plusieurs empreintes, essayées dans l'ordre.

Ces mécanismes reprennent les principes de [TwitchDropsMiner](https://github.com/DevilXD/TwitchDropsMiner), [Twitch-Channel-Points-Miner-v2](https://github.com/rdavydov/Twitch-Channel-Points-Miner-v2) et [twitch-drops-miner](https://github.com/ohne-b/twitch-drops-miner). Aucun code n'en est copié.

Utiliser ces outils va contre l'esprit des conditions d'utilisation de Twitch : un compte peut être sanctionné.

## Organisation

- `src-tauri/src/twitch/` : connexion (`auth.rs`), requêtes GQL (`gql.rs`), PubSub (`pubsub.rs`), visionnage simulé (`watch.rs`), présence dans le chat (`chat.rs`).
- `src-tauri/src/streamelements.rs` : StreamElements (solde, giveaways, tickets).
- `src-tauri/src/drops.rs` : drops (campagnes, choix du stream, réclamation).
- `src-tauri/src/giveaways/` : moniteur des chats, détection (`detect.rs`), lecture des bots (`bots.rs`), participation.
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
