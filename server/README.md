# Twitch Desk Sync

Serveur de synchronisation de Twitch Desk : plusieurs personnes mettent en commun ce qu'elles ajoutent dans l'application (cadeaux, groupes, giveaways, chaînes surveillées, fiches des streamers, formulations de gagnant et bots ajoutés).

Chaque application envoie ses modifications et récupère celles des autres toutes les 30 secondes (et quelques secondes après une modification). Depuis la 0.5, le serveur prévient aussi les applications en temps réel dès qu'une modification arrive : elles la récupèrent tout de suite (le passage régulier s'espace alors à quelques minutes). La dernière modification reçue l'emporte ; une suppression s'applique chez tout le monde. Deux personnes qui créent le même élément (même cadeau, même groupe, même giveaway ouvert sur une chaîne) se retrouvent avec un seul élément.

Ce qui reste propre à chacun : comptes Twitch, participations, gains et lots reçus, points, drops, réglages.

Dans l'application, chacun choisit parmi les données proposées par le serveur celles qu'il synchronise (Paramètres → Synchronisation). Les chaînes surveillées sont décochées par défaut : chacun garde sa liste tant qu'il ne coche pas la case.

Depuis la 0.6, le serveur sert aussi une **web app mobile** (`/app`) : sur le téléphone, les giveaways ouverts et en pause, la commande à copier, les lots, les conditions, l'heure du tirage, le statut live de la chaîne et les comptes de *ton* PC qui ont déjà participé. Elle ne modifie rien (lecture seule). Depuis la 0.7, elle reçoit aussi les alertes du PC en **notifications push**.

## Installation (Portainer)

Stacks → Add stack → **Repository** :

- Repository URL : `https://github.com/powange/twitch-desk`
- Compose path : `server/docker-compose.yml`
- Environment variables : `SYNC_CODE` (obligatoire), et au besoin `SYNC_COLLECTIONS`, `SYNC_HISTORY_DAYS`, `SYNC_PROXY_HOPS`, `SYNC_HOST_PORT` (port publié sur l'hôte, `8787` par défaut), `SYNC_HOST_BIND` (interface de l'hôte où il est publié, `127.0.0.1` par défaut)

L'image `ghcr.io/powange/twitch-desk-sync` est publiée par GitHub Actions à chaque modification du serveur ou de la web app. Pour la construire sur place à la place, remplacer `image:` par le bloc `build:` commenté dans le compose (contexte : la racine du dépôt, `dockerfile: server/Dockerfile`), ou à la main : `docker build -f server/Dockerfile -t twitch-desk-sync .` depuis la racine du dépôt.

Mettre le serveur derrière un proxy HTTPS (Nginx Proxy Manager, Traefik, Caddy…) : le code circule dans chaque requête. Le compose rejoint le réseau `proxy-network` : le proxy joint le serveur par `twitch-desk-sync:8787`.

Le port publié sur l'hôte sert en HTTP simple : il n'est ouvert qu'à l'hôte lui-même (`127.0.0.1`) par défaut, pour un proxy installé sur l'hôte ou un essai local. Pour l'ouvrir aux autres machines (proxy sur une autre machine, réseau local), définir `SYNC_HOST_BIND=0.0.0.0` (ou l'adresse d'une interface), en sachant que le code y passe en clair ; joint ainsi sans proxy, mettre aussi `SYNC_PROXY_HOPS=0`.

Protection : après 10 codes incorrects en 10 minutes depuis une même adresse, celle-ci est refusée (réponse 429) pendant 15 minutes ; une adresse IPv6 compte pour tout son réseau /64. Derrière un proxy, l'adresse du client est celle que le proxy ajoute à `X-Forwarded-For` (`SYNC_PROXY_HOPS` : nombre de proxys en chaîne, 1 par défaut). Le code est vérifié avant la lecture du corps de la requête ; 16 requêtes au plus sont traitées en même temps, chacune abandonnée après 45 s. Les flux temps réel (`/api/v1/events`) n'entrent pas dans cette limite : 64 au plus restent ouverts en même temps (au-delà, réponse 503 ; l'application réessaie et garde en attendant son passage toutes les 30 s), dont 2 par téléphone et 4 par adresse pour les applications : au-delà, le plus ancien est fermé.

### Temps réel derrière un proxy

`/api/v1/events` est une connexion qui reste ouverte (Server-Sent Events) : le proxy doit transmettre chaque événement sans le garder en mémoire tampon, et ne pas couper une connexion calme trop tôt (le serveur envoie un commentaire toutes les 20 s).

- Nginx Proxy Manager, Traefik, Caddy : rien à régler d'habitude (le serveur envoie `X-Accel-Buffering: no`, que Nginx respecte).
- Nginx écrit à la main, au besoin :

  ```nginx
  location /api/v1/events {
      proxy_pass http://twitch-desk-sync:8787;
      proxy_http_version 1.1;
      proxy_buffering off;
      proxy_read_timeout 1h;
  }
  ```

- Cloudflare et autres CDN : les flux longs peuvent être coupés ; l'application se reconnecte seule.

Sans temps réel (serveur d'avant la 0.5 ou proxy qui bloque le flux), l'application continue de synchroniser toutes les 30 s.

## Web app mobile (0.6)

### Connecter un téléphone

Dans l'application : Paramètres → Synchronisation → « Connecter un téléphone ». Un code à 6 chiffres s'affiche avec un QR code (lien `https://<serveur>/app/#pair=<code>`) : le scanner avec l'appareil photo ouvre la web app, qui se connecte seule ; sinon ouvrir `https://<serveur>/app/` et taper le code. Le téléphone apparaît alors dans la liste « Téléphones connectés », d'où on peut le déconnecter. Sur le téléphone, « Ajouter à l'écran d'accueil » l'installe comme une application.

- Le code vaut 5 minutes, une seule fois ; un nouveau code remplace le précédent. Après 5 codes incorrects en 10 minutes, une adresse est refusée pendant 10 minutes ; après 30 codes incorrects en 10 minutes toutes adresses confondues, les codes en attente sont annulés (en demander un nouveau sur le PC).
- Le téléphone reçoit son propre jeton (le code du serveur ne quitte jamais le PC). Le serveur n'en garde que l'empreinte (SHA-256). Ce jeton ne permet que de lire les giveaways ouverts et l'état publié par le PC qui l'a associé, et de gérer son propre abonnement aux notifications : rien d'autre.
- Déconnecter un téléphone depuis le PC refuse son jeton aussitôt et ferme sa connexion temps réel.
- Chaque PC publie un petit état (comptes, participations aux giveaways ouverts, statut live de leurs chaînes) quelques secondes après un changement : le téléphone se met à jour en temps réel.

### Derrière un proxy

La web app est servie sous `/app` sur le même port que l'API : avec un seul hôte (`https://sync.exemple.fr`), rien à régler, `/app` et `/api` passent par le même proxy.

Le proxy doit transmettre `/app` **avec son préfixe** (`/app/…` arrive tel quel au serveur ; pas de réécriture vers `/`). La web app appelle l'API sur la même adresse (`/api/v1/…`) : `/api` doit donc rester joignable sur le même hôte, vers le port de l'API.

Port à part (`SYNC_WEB_PORT`, facultatif) : le serveur sert alors aussi la web app, et seulement elle, sur ce second port (sa racine `/` redirige vers `/app/`). Utile pour router `/app` vers un autre port que l'API, par exemple avec Nginx Proxy Manager :

- Proxy host `sync.exemple.fr` → `twitch-desk-sync:8787` (l'API, comme avant, SSL compris)
- Onglet « Custom locations » : location `/app`, scheme `http`, forward hostname `twitch-desk-sync`, forward port `8788` (la valeur de `SYNC_WEB_PORT`). Ne rien mettre après le nom d'hôte (pas de `/` final dans « Forward Hostname / IP ») : NPM garde alors le préfixe `/app`.

En Nginx écrit à la main :

```nginx
location /app/ {
    proxy_pass http://twitch-desk-sync:8788;   # sans chemin après le port : /app/ est gardé
}
```

Si la web app est publiée à une autre adresse que celle du serveur, l'indiquer dans l'application (« Adresse de la web app ») : c'est elle que le QR code contient. Elle doit rester sur le même hôte que `/api`.

Le flux temps réel du téléphone (`/api/v1/events`) passe par les mêmes règles que celui des applications (voir plus haut). Le jeton du téléphone voyage dans l'en-tête `Authorization`, jamais dans l'adresse : il n'apparaît pas dans les journaux du proxy.

### Notifications push (0.7)

Depuis la web app (menu ⋮ → Notifications), un téléphone s'abonne aux alertes de son choix : giveaway détecté, gain, rappel de tirage, mention, scanner, drop obtenu (0.8.1). L'application du PC envoie chaque alerte au serveur, qui la transmet aux téléphones abonnés (Web Push), même web app fermée.

- La web app doit être servie en **HTTPS** (exigence des navigateurs).
- Sur iPhone et iPad (iOS 16.4 ou plus) : ajouter d'abord la web app à l'écran d'accueil, puis activer les notifications depuis cette icône.
- Le serveur doit pouvoir joindre Internet en sortie (services de push de Google, Apple, Mozilla, Microsoft). Seuls ces services sont acceptés comme adresse d'abonnement, et aucune redirection n'est suivie : le serveur n'envoie rien ailleurs, ni vers son réseau local.
- Images des lots : une image intégrée au cadeau est servie par le serveur ; une image en ligne n'est montrée aux téléphones que si elle vient de Twitch (sinon son hébergeur verrait leur adresse IP).
- Une clé VAPID est créée au premier démarrage et gardée dans la base : la conserver (volume `/data`), sinon les téléphones devront réactiver leurs notifications.
- Le contenu de chaque notification est chiffré pour le téléphone : le service de push ne le lit pas. Déconnecter un téléphone efface aussi son abonnement.

### Pages Points de chaîne, StreamElements et Drops, achats depuis le téléphone (0.8)

La web app affiche aussi, pour chaque compte du PC, ses chaînes de points de chaîne (état, solde, gain du jour), ses chaînes StreamElements (points, gain du jour, giveaway en cours) et ses drops (chaîne regardée, campagnes en cours, progression de chaque drop), d'après l'état publié par le PC. Le téléphone relit cet état seul (`GET /api/v1/app/state`), sans recharger les giveaways ni leurs images.

Depuis la page StreamElements, un téléphone peut demander l'achat de tickets. Le serveur garde la demande (5 minutes au plus) et l'annonce au PC par le flux temps réel (événement `action`) ; le PC l'exécute avec son propre jeton StreamElements, puis rend le résultat, annoncé au téléphone. Le serveur ne détient aucun jeton. Les achats ne sont possibles que si le PC les autorise (Paramètres → Synchronisation → « Autoriser les achats depuis le téléphone », désactivé par défaut) et pendant qu'il est ouvert.

Depuis le menu ⋮, un téléphone peut aussi demander au PC de vérifier ses mises à jour, puis d'installer la nouvelle version (0.8.2) : le PC la télécharge (paquet signé, refusé sinon), l'installe et redémarre. Pas besoin d'autorisation pour cela : seule la version officielle publiée sur GitHub peut être installée.

## Variables d'environnement

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `SYNC_CODE` | (obligatoire) | Code à donner aux utilisateurs, 8 caractères au moins |
| `SYNC_COLLECTIONS` | toutes | Données partagées, séparées par des virgules : `groups`, `gifts`, `giveaways`, `channels`, `streamers`, `detection` (formulations de gagnant et bots ajoutés) |
| `SYNC_HISTORY_DAYS` | `90` | Conservation de l'historique des modifications (qui a modifié quel document et quand ; le contenu n'y est plus gardé depuis la 0.4) |
| `SYNC_PROXY_HOPS` | `1` | Proxys devant le serveur, pour lire l'adresse du client dans `X-Forwarded-For` ; `0` si le serveur est joint directement, `2` avec Cloudflare devant le proxy (sinon l'adresse d'un nœud Cloudflare serait prise pour celle du client, et un seul fautif bloquerait tous ceux qui passent par lui). Le nombre d'adresses reçues est écrit une fois dans le journal au démarrage |
| `SYNC_DB` | `/data/sync.db` | Fichier SQLite (volume `/data`) |
| `SYNC_PORT` | `8787` | Port d'écoute |
| `SYNC_WEB_PORT` | (aucun) | Port supplémentaire qui ne sert que la web app (`/app`) ; sans lui, elle est servie sur `SYNC_PORT` seulement |
| `SYNC_PUSH_CONTACT` | `https://github.com/powange/twitch-desk` | Contact donné aux services de push dans la signature VAPID (`mailto:…` ou `https://…`). À remplacer par un contact de l'hébergeur (`mailto:toi@exemple.fr`) : par défaut, Google, Apple ou Mozilla contacteraient l'auteur du projet en cas de problème, et un avertissement est écrit au démarrage |
| `SYNC_WEB_DIR` | `/usr/share/twitch-desk-sync/web` | Fichiers de la web app (remplis dans l'image) |
| `SYNC_HOST_BIND` | `127.0.0.1` | (compose) Interface de l'hôte où le port est publié |
| `SYNC_HOST_PORT` | `8787` | (compose) Port publié sur l'hôte |

Les giveaways partagés sont ceux encore ouverts ou détectés depuis moins de 60 jours.

## Dans l'application

Paramètres → Synchronisation : adresse du serveur (`https://sync.exemple.fr`) et code, puis « Tester la connexion » et activer.

## API

Sauf mention contraire, les routes demandent `Authorization: Bearer <SYNC_CODE>`. Un téléphone associé utilise `Authorization: Device <jeton>`, accepté seulement par les routes `/api/v1/app/…` et `/api/v1/events` ; son flux n'annonce que les modifications des giveaways, cadeaux et groupes (ce que montre sa vue).

- `GET /health` : `ok` (le serveur et sa base répondent), sinon 503
- `GET /api/v1/info` : version, collections partagées, numéro de la dernière modification
- `GET /api/v1/changes?since=<seq>&limit=<n>` : documents modifiés depuis `seq` (dernière version de chacun, suppressions comprises), `more` s'il en reste (page de 16 Mo de contenu au plus)
- `POST /api/v1/changes` (16 Mo au plus) : `{ "author": "pseudo", "changes": [{ "collection": "gifts", "id": "…", "data": { … } }, { "collection": "gifts", "id": "…", "deleted": true }] }`
- `GET /api/v1/events` (0.5 et plus ; code ou jeton d'appareil) : flux Server-Sent Events ; à l'ouverture puis à chaque modification enregistrée, `event: seq` avec `data: {"seq": <numéro de la dernière modification>}` (des modifications rapprochées peuvent n'en donner qu'un) ; un commentaire toutes les 20 s garde la connexion ouverte. 503 quand 64 flux sont déjà ouverts. Pour un téléphone (0.6), aussi `event: state` avec `data: {"instance": "<id>"}` quand son PC publie un nouvel état ; le flux se ferme quand il est déconnecté

Web app (0.6 et plus) :

- `PUT /api/v1/instances/{id}` : `{ "name": "PC du salon" }`, enregistre l'application (identifiant tiré une fois par elle)
- `PUT /api/v1/instances/{id}/state` (256 Ko au plus) : état publié par l'application, objet JSON (`accounts`, `giveaways` : `{ "<id partagé>": { "participated": [...], "handled": true } }`, `live` : `{ "<chaîne>": { "live": true, "viewers": 42, "category": "…", "title": "…" } }`) ; 204
- `POST /api/v1/pair/start` : `{ "instance": "<id>", "name": "…" }` → `{ "code": "123456", "expiresAt": "…", "expiresIn": 300 }`
- `POST /api/v1/pair/claim` (sans code ; 5 codes incorrects en 10 min par adresse, puis 429 pendant 10 min) : `{ "code": "123456", "deviceName": "iPhone" }` → `{ "token": "…", "deviceId": "…", "instance": { "id": "…", "name": "…" } }`
- `GET /api/v1/instances/{id}/devices` : `[{ "id", "name", "createdAt", "lastSeen" }]`
- `DELETE /api/v1/instances/{id}/devices/{deviceId}` : révoque le téléphone (204, 404 s'il est inconnu)
- `GET /api/v1/app/overview` (jeton d'appareil) : `{ instance, device, state, stateAt, giveaways, seq }` ; giveaways ouverts puis en pause (les plus récents d'abord), cadeaux remplacés par `{ id, title, image, gameName, group }` (`image` : adresse ; une image intégrée au cadeau est servie à part, voir la route suivante)
- `GET /api/v1/app/gifts/{id}/image?v=<empreinte>` (jeton d'appareil) : image intégrée d'un cadeau (PNG, JPEG, WebP, GIF ou AVIF), en cache privé un an ; l'empreinte change avec l'image
- `GET /app/…` (sans authentification) : fichiers de la web app ; `/app/assets/…` en cache un an (noms à empreinte), le reste revérifié à chaque visite ; une adresse sans extension renvoie `index.html`

À la première ouverture par la 0.4, une base existante est compactée une fois (`VACUUM`) après que le contenu est retiré de l'historique : le démarrage peut prendre quelques secondes de plus.
