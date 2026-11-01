# Twitch Desk Sync

Serveur de synchronisation de Twitch Desk : plusieurs personnes mettent en commun ce qu'elles ajoutent dans l'application (cadeaux, groupes, giveaways, chaînes surveillées, fiches des streamers).

Chaque application envoie ses modifications et récupère celles des autres toutes les 30 secondes (et quelques secondes après une modification). La dernière modification reçue l'emporte ; une suppression s'applique chez tout le monde. Deux personnes qui créent le même élément (même cadeau, même groupe, même giveaway ouvert sur une chaîne) se retrouvent avec un seul élément.

Ce qui reste propre à chacun : comptes Twitch, participations, gains et lots reçus, points, drops, réglages.

## Installation (Portainer)

Stacks → Add stack → **Repository** :

- Repository URL : `https://github.com/powange/twitch-desk`
- Compose path : `server/docker-compose.yml`
- Environment variables : `SYNC_CODE` (obligatoire), et au besoin `SYNC_COLLECTIONS`, `SYNC_HISTORY_DAYS`, `SYNC_HOST_PORT` (port publié sur l'hôte, `8787` par défaut)

L'image `ghcr.io/powange/twitch-desk-sync` est publiée par GitHub Actions à chaque modification du serveur. Pour la construire sur place à la place, remplacer `image:` par `build: .` dans le compose.

Mettre le serveur derrière un proxy HTTPS (Nginx Proxy Manager, Traefik, Caddy…) : le code circule dans chaque requête.

## Variables d'environnement

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `SYNC_CODE` | (obligatoire) | Code à donner aux utilisateurs, 8 caractères au moins |
| `SYNC_COLLECTIONS` | toutes | Données partagées, séparées par des virgules : `groups`, `gifts`, `giveaways`, `channels`, `streamers` |
| `SYNC_HISTORY_DAYS` | `90` | Conservation de l'historique des modifications |
| `SYNC_DB` | `/data/sync.db` | Fichier SQLite (volume `/data`) |
| `SYNC_PORT` | `8787` | Port d'écoute |

Les giveaways partagés sont ceux encore ouverts ou détectés depuis moins de 60 jours.

## Dans l'application

Paramètres → Synchronisation : adresse du serveur (`https://sync.exemple.fr`) et code, puis « Tester la connexion » et activer.

## API

Toutes les routes sauf `/health` demandent `Authorization: Bearer <SYNC_CODE>`.

- `GET /health` : `ok`
- `GET /api/v1/info` : version, collections partagées, numéro de la dernière modification
- `GET /api/v1/changes?since=<seq>&limit=<n>` : documents modifiés depuis `seq` (dernière version de chacun, suppressions comprises), `more` s'il en reste
- `POST /api/v1/changes` : `{ "author": "pseudo", "changes": [{ "collection": "gifts", "id": "…", "data": { … } }, { "collection": "gifts", "id": "…", "deleted": true }] }`
