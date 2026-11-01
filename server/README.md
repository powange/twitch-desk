# Twitch Desk Sync

Serveur de synchronisation de Twitch Desk : plusieurs personnes mettent en commun ce qu'elles ajoutent dans l'application (cadeaux, groupes, giveaways, chaînes surveillées, fiches des streamers, formulations de gagnant et bots ajoutés).

Chaque application envoie ses modifications et récupère celles des autres toutes les 30 secondes (et quelques secondes après une modification). La dernière modification reçue l'emporte ; une suppression s'applique chez tout le monde. Deux personnes qui créent le même élément (même cadeau, même groupe, même giveaway ouvert sur une chaîne) se retrouvent avec un seul élément.

Ce qui reste propre à chacun : comptes Twitch, participations, gains et lots reçus, points, drops, réglages.

Dans l'application, chacun choisit parmi les données proposées par le serveur celles qu'il synchronise (Paramètres → Synchronisation). Les chaînes surveillées sont décochées par défaut : chacun garde sa liste tant qu'il ne coche pas la case.

## Installation (Portainer)

Stacks → Add stack → **Repository** :

- Repository URL : `https://github.com/powange/twitch-desk`
- Compose path : `server/docker-compose.yml`
- Environment variables : `SYNC_CODE` (obligatoire), et au besoin `SYNC_COLLECTIONS`, `SYNC_HISTORY_DAYS`, `SYNC_PROXY_HOPS`, `SYNC_HOST_PORT` (port publié sur l'hôte, `8787` par défaut), `SYNC_HOST_BIND` (interface de l'hôte où il est publié, `127.0.0.1` par défaut)

L'image `ghcr.io/powange/twitch-desk-sync` est publiée par GitHub Actions à chaque modification du serveur. Pour la construire sur place à la place, remplacer `image:` par `build: .` dans le compose.

Mettre le serveur derrière un proxy HTTPS (Nginx Proxy Manager, Traefik, Caddy…) : le code circule dans chaque requête. Le compose rejoint le réseau `proxy-network` : le proxy joint le serveur par `twitch-desk-sync:8787`.

Le port publié sur l'hôte sert en HTTP simple : il n'est ouvert qu'à l'hôte lui-même (`127.0.0.1`) par défaut, pour un proxy installé sur l'hôte ou un essai local. Pour l'ouvrir aux autres machines (proxy sur une autre machine, réseau local), définir `SYNC_HOST_BIND=0.0.0.0` (ou l'adresse d'une interface), en sachant que le code y passe en clair ; joint ainsi sans proxy, mettre aussi `SYNC_PROXY_HOPS=0`.

Protection : après 10 codes incorrects en 10 minutes depuis une même adresse, celle-ci est refusée (réponse 429) pendant 15 minutes. Derrière un proxy, l'adresse du client est celle que le proxy ajoute à `X-Forwarded-For` (`SYNC_PROXY_HOPS` : nombre de proxys en chaîne, 1 par défaut). Le code est vérifié avant la lecture du corps de la requête ; 16 requêtes au plus sont traitées en même temps, chacune abandonnée après 45 s.

## Variables d'environnement

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `SYNC_CODE` | (obligatoire) | Code à donner aux utilisateurs, 8 caractères au moins |
| `SYNC_COLLECTIONS` | toutes | Données partagées, séparées par des virgules : `groups`, `gifts`, `giveaways`, `channels`, `streamers`, `detection` (formulations de gagnant et bots ajoutés) |
| `SYNC_HISTORY_DAYS` | `90` | Conservation de l'historique des modifications (qui a modifié quel document et quand ; le contenu n'y est plus gardé depuis la 0.4) |
| `SYNC_PROXY_HOPS` | `1` | Proxys devant le serveur, pour lire l'adresse du client dans `X-Forwarded-For` ; `0` si le serveur est joint directement |
| `SYNC_DB` | `/data/sync.db` | Fichier SQLite (volume `/data`) |
| `SYNC_PORT` | `8787` | Port d'écoute |
| `SYNC_HOST_BIND` | `127.0.0.1` | (compose) Interface de l'hôte où le port est publié |
| `SYNC_HOST_PORT` | `8787` | (compose) Port publié sur l'hôte |

Les giveaways partagés sont ceux encore ouverts ou détectés depuis moins de 60 jours.

## Dans l'application

Paramètres → Synchronisation : adresse du serveur (`https://sync.exemple.fr`) et code, puis « Tester la connexion » et activer.

## API

Toutes les routes sauf `/health` demandent `Authorization: Bearer <SYNC_CODE>`.

- `GET /health` : `ok` (le serveur et sa base répondent), sinon 503
- `GET /api/v1/info` : version, collections partagées, numéro de la dernière modification
- `GET /api/v1/changes?since=<seq>&limit=<n>` : documents modifiés depuis `seq` (dernière version de chacun, suppressions comprises), `more` s'il en reste (page de 16 Mo de contenu au plus)
- `POST /api/v1/changes` (16 Mo au plus) : `{ "author": "pseudo", "changes": [{ "collection": "gifts", "id": "…", "data": { … } }, { "collection": "gifts", "id": "…", "deleted": true }] }`

À la première ouverture par la 0.4, une base existante est compactée une fois (`VACUUM`) après que le contenu est retiré de l'historique : le démarrage peut prendre quelques secondes de plus.
