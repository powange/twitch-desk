// Changelog affiché dans Paramètres → Changelog, écrit pour les utilisateurs (pas pour les
// développeurs). À compléter à chaque version, la plus récente en premier :
// « new » : nouveauté, « improved » : modification ou amélioration, « fixed » : problème réglé.

export type ChangeKind = 'new' | 'improved' | 'fixed'

export interface Release {
  version: string
  date: string
  // Résumé en une phrase (facultatif)
  title?: string
  changes: { kind: ChangeKind, text: string }[]
}

export const CHANGELOG: Release[] = [
  {
    version: '0.48.18',
    date: '2026-11-04',
    changes: [
      { kind: 'new', text: 'Giveaways Lingo Echo : quand le bot Lingo_Echo poste le lien de son giveaway (lingo-echo.eu) sur une chaîne surveillée, un giveaway par commande est créé, ou le lien est ajouté à celui en cours. La commande est remplie dès que la détection la repère dans le chat. Un badge « Lingo Echo » le signale dans la file d\'alertes et les tableaux.' }
    ]
  },
  {
    version: '0.48.17',
    date: '2026-11-04',
    changes: [
      { kind: 'fixed', text: 'Annonces du bot Lingo Echo reconnues : « La personne gagnante est : @pseudo … chance de gagner : 12.50% » enregistre le gagnant et sa chance sur le giveaway de la chaîne. La même tournure écrite par un autre bot modérateur est aussi reconnue.' }
    ]
  },
  {
    version: '0.48.16',
    date: '2026-11-04',
    changes: [
      { kind: 'fixed', text: 'Page Chaînes : les conditions des fiches de streamers reçues d\'un autre PC par la synchronisation s\'affichent sans avoir à changer de page.' }
    ]
  },
  {
    version: '0.48.15',
    date: '2026-11-04',
    changes: [
      { kind: 'fixed', text: 'Filtre « Streamer » des giveaways passés : la liste des streamers s\'affiche enfin (elle restait vide à cause de l\'entrée « Tous les streamers »), et les lettres tapées au milieu du nom les trouvent (« lf_ », « mav »).' },
      { kind: 'improved', text: 'Filtres « Streamer » (page Chaînes et giveaways passés) : photo de profil du streamer à gauche de son nom, quand elle est connue.' },
      { kind: 'fixed', text: 'Formulaire de giveaway : la lecture du giveaway StreamElements d\'après son lien pouvait ne pas se faire (erreur au chargement du formulaire).' }
    ]
  },
  {
    version: '0.48.14',
    date: '2026-11-04',
    changes: [
      { kind: 'new', text: 'Page Chaînes : filtre « Streamers » à choix multiple, avec recherche. Un bout de nom suffit (« mav » trouve lf_maverick), et « Contient « mav » » garde toutes les chaînes dont le nom contient ce bout.' },
      { kind: 'fixed', text: 'Filtres des giveaways passés : la recherche du streamer trouve maintenant les noms qui contiennent les lettres tapées, même au milieu du nom. Si la liste des streamers ne peut pas être lue, un message le dit.' }
    ]
  },
  {
    version: '0.48.13',
    date: '2026-11-04',
    changes: [
      { kind: 'improved', text: 'Fenêtre des nouveautés après une mise à jour : la dernière version vue est gardée dans la base de l\'application (elle ne se perd plus), et retrouvée dans les journaux si besoin. Toutes les versions sautées sont affichées.' }
    ]
  },
  {
    version: '0.48.12',
    date: '2026-11-04',
    title: 'Corrections du rapport et nouveautés affichées après une mise à jour',
    changes: [
      { kind: 'new', text: 'Après une mise à jour, une fenêtre montre une fois les nouveautés de toutes les versions arrivées depuis ta dernière ouverture.' },
      { kind: 'improved', text: 'Filtres des giveaways passés : « Streamer » accepte aussi un bout de nom. Tape « mav » et choisis « Contient « mav » » pour retrouver lf_maverick.' },
      { kind: 'fixed', text: 'Deux giveaways ouverts sur une chaîne : le 2e gagnant annoncé dans les 15 minutes d\'un tirage va dans le giveaway tiré, au lieu de fermer l\'autre.' },
      { kind: 'fixed', text: 'Retirer les gagnants d\'un giveaway le rouvre vraiment (ouvert, ou en pause si la chaîne est hors ligne) au lieu d\'être refermé dans la minute.' },
      { kind: 'fixed', text: '« Annuler la réponse » d\'une notification restée d\'un lancement précédent n\'annule plus une autre réponse, et ne démarre plus l\'application si elle était fermée.' },
      { kind: 'fixed', text: 'Achat de récompense de points de chaîne : un nouvel essai après une réponse perdue de Twitch n\'est plus payé deux fois, et le message indique un achat incertain à vérifier.' },
      { kind: 'fixed', text: 'Alertes du téléphone : une alerte non transmise (coupure, serveur redémarré) est renvoyée toutes les 30 secondes pendant 15 minutes au lieu d\'être perdue.' },
      { kind: 'fixed', text: 'Ancien giveaway : les changements faits dans la fiche du streamer (Discord, notes, commandes ignorées, réponse automatique) sont enregistrés.' },
      { kind: 'fixed', text: 'Le badge StreamElements d\'un compte ouvre la page StreamElements sur ce compte, et plus sur celui choisi auparavant.' },
      { kind: 'fixed', text: 'Bouton « Participer » du tableau : il participe avec tous les comptes qui ne l\'ont pas encore fait, au lieu de ne rien faire parfois.' },
      { kind: 'fixed', text: 'Chance de gagner invalide : refusée avant l\'envoi, et le message ne dit plus « non enregistré » quand seuls les gagnants ont été refusés.' },
      { kind: 'fixed', text: 'Récompenses de points de chaîne du formulaire : relues à chaque ouverture et après une erreur, et vidées dès qu\'on change de chaîne.' },
      { kind: 'fixed', text: 'Téléphone associé de nouveau (nouveau QR code) : l\'ancien appareil est remplacé, plus de notifications en double.' }
    ]
  },
  {
    version: '0.48.11',
    date: '2026-11-04',
    changes: [
      { kind: 'improved', text: 'File d\'alertes : chaque carte précise que le giveaway est déjà ajouté aux giveaways en cours. Le bouton « Ignorer » est remplacé par « Vu », tout à droite comme pour les mentions : il retire seulement la carte de la file, le giveaway reste en cours.' },
      { kind: 'new', text: 'Carte d\'un giveaway StreamElements : pour chaque compte, ses tickets s\'il relève déjà les points StreamElements de la chaîne, sinon un bouton pour l\'ajouter à sa page StreamElements. « Acheter des tickets » ouvre la page StreamElements filtrée sur la chaîne.' }
    ]
  },
  {
    version: '0.48.10',
    date: '2026-11-04',
    changes: [
      { kind: 'improved', text: 'Réponse automatique si tu gagnes : elle part au moment du tirage plus le délai réglé dans la fiche du streamer, à 10 secondes près. Si le gain n\'est connu que plus tard (en relisant StreamElements), le délai restant est raccourci d\'autant, et la réponse n\'est pas envoyée si ce moment est passé de plus de 10 secondes.' }
    ]
  },
  {
    version: '0.48.9',
    date: '2026-11-04',
    changes: [
      { kind: 'improved', text: 'Condition « Avoir dit bonjour » : impossible de dire bonjour à une chaîne hors live (un bonjour ne compte que pendant le live). Un message l\'explique au lieu d\'ouvrir la fenêtre d\'envoi.' },
      { kind: 'improved', text: 'Fenêtre « Dire bonjour » : une case par compte pour choisir ceux qui écrivent, et une phrase différente pour chaque compte (le dé en tire une que les autres n\'ont pas). L\'envoi est bloqué si deux comptes ont le même message.' },
      { kind: 'fixed', text: 'Giveaways StreamElements : l\'heure du tirage donnée par StreamElements est maintenant enregistrée. La réponse automatique n\'est écrite que pour un tirage de moins de 5 minutes : si l\'application était fermée pendant le tirage, plus de remerciement envoyé dans le chat des heures après.' }
    ]
  },
  {
    version: '0.48.8',
    date: '2026-11-04',
    changes: [
      { kind: 'improved', text: 'Condition « Avoir dit bonjour » : un clic n\'envoie plus de message tout de suite. Une fenêtre montre la phrase tirée au hasard pour chaque compte qui ne l\'a pas encore dit : tu peux la modifier, en tirer une autre (dé), puis confirmer l\'envoi.' },
      { kind: 'new', text: 'Chaînes surveillées : quand une chaîne est en live dans une de tes catégories et qu\'un giveaway StreamElements y est en cours, il est créé tout seul (avec sa commande et son maximum de tickets), même si aucun bot n\'a posté son lien. Il est cherché toutes les 3 minutes ; un giveaway que tu as clos n\'est jamais recréé.' },
      { kind: 'improved', text: 'Réponse automatique si tu gagnes : au survol de son icône, une fenêtre affiche le message en entier, avec son délai, au lieu d\'une infobulle tronquée.' }
    ]
  },
  {
    version: '0.48.7',
    date: '2026-11-04',
    changes: [
      { kind: 'improved', text: 'Filtres des giveaways passés : « Streamer » est maintenant une liste avec recherche, qui ne propose que les streamers présents dans tes giveaways passés.' }
    ]
  },
  {
    version: '0.48.6',
    date: '2026-11-04',
    changes: [
      { kind: 'new', text: 'StreamElements, « Achat auto » : sous l\'interrupteur, un plafond de tickets à ne pas dépasser sur un giveaway (0 : sans limite). Il s\'ajoute au maximum par personne fixé par le streamer : c\'est le plus petit des deux qui compte.' },
      { kind: 'improved', text: 'Page Alertes : la commande d\'un giveaway s\'affiche avec, juste en dessous, le lien du giveaway.' },
      { kind: 'new', text: 'Page Alertes, colonne Participations d\'un giveaway StreamElements en cours : pour un compte qui ne relève pas encore les points StreamElements de la chaîne, un bouton l\'ajoute à sa page StreamElements. Sinon, ses tickets sont affichés (0 s\'il n\'en a pas encore acheté).' }
    ]
  },
  {
    version: '0.48.5',
    date: '2026-11-04',
    title: 'Campagnes de drops ignorées et achat auto en une fois',
    changes: [
      { kind: 'new', text: 'Page Drops : bouton « Ignorer » sur chaque campagne. Une campagne ignorée n\'est plus regardée ni réclamée, ne déclenche plus de notification et disparaît de la liste et du téléphone. La case « Afficher les ignorées » les fait réapparaître, avec un bouton « Rétablir ».' },
      { kind: 'improved', text: 'StreamElements, « Achat auto » : tous les tickets que ton solde permet sont achetés en une fois, sans dépasser le maximum par personne. Une fois ce maximum atteint, plus aucune requête n\'est faite pour ce giveaway. Sans maximum fixé par le streamer, tout le solde possible est utilisé.' },
      { kind: 'improved', text: 'Drops : un jeu n\'est plus marqué « Terminé » quand une autre de ses campagnes a encore des drops à obtenir.' },
      { kind: 'improved', text: 'Page Alertes : quand la commande d\'un giveaway est connue, une petite icône à côté ouvre aussi le lien du giveaway.' }
    ]
  },
  {
    version: '0.48.4',
    date: '2026-11-04',
    changes: [
      { kind: 'improved', text: 'Formulaire de giveaway : les informations d\'une chaîne déjà ouverte (encart du streamer, Discord, fiche) s\'affichent tout de suite, puis sont mises à jour en arrière-plan, avec un petit indicateur en haut à droite de l\'encart.' }
    ]
  },
  {
    version: '0.48.3',
    date: '2026-11-04',
    changes: [
      { kind: 'fixed', text: 'Giveaways passés : le cœur « Follow requis » indique maintenant si tes comptes suivent la chaîne (plein et vert si tous la suivent, cliquable pour suivre sinon). Avant, l\'état restait inconnu pour une chaîne sans giveaway en cours.' }
    ]
  },
  {
    version: '0.48.2',
    date: '2026-11-04',
    title: 'Ancienneté du follow en heures ou minutes',
    changes: [
      { kind: 'improved', text: 'Conditions d\'un giveaway et fiche du streamer : l\'ancienneté du follow s\'écrit maintenant comme une durée, par exemple « 7j », « 12h », « 30min », « 45s » ou « 2h 30min » (j : jours, h : heures, min : minutes, s : secondes). Les durées déjà enregistrées en jours sont gardées.' },
      { kind: 'fixed', text: 'Giveaway StreamElements : en collant le lien du giveaway, « Tickets maximum par personne » est rempli avec la limite fixée sur StreamElements (si le champ est vide).' }
    ]
  },
  {
    version: '0.48.1',
    date: '2026-11-04',
    title: 'Gagnants StreamElements et giveaways passés',
    changes: [
      { kind: 'fixed', text: 'Quand le bot StreamElements annonce plusieurs gagnants d\'affilée pendant qu\'un giveaway StreamElements est suivi, tous les gagnants sont maintenant gardés, et plus seulement le dernier.' },
      { kind: 'fixed', text: 'Si StreamElements ne répond pas, un gagnant annoncé n\'est plus rangé sur le mauvais giveaway (et tu n\'as plus deux « Tu as gagné »). L\'annonce attend que StreamElements réponde, pendant 8 minutes au plus.' },
      { kind: 'fixed', text: 'Le tableau des giveaways passés se met à jour après l\'ajout d\'un ancien giveaway, un import par lot ou une suppression.' },
      { kind: 'fixed', text: '« Ancien giveaway » n\'est plus pré-rempli avec le giveaway du jour de la chaîne (type, lien, commande et cadeaux suggérés).' }
    ]
  },
  {
    version: '0.48.0',
    date: '2026-11-03',
    title: 'Achat automatique des tickets StreamElements',
    changes: [
      { kind: 'new', text: 'Page StreamElements : colonne « Achat auto » pour chaque streamer. Pendant un giveaway StreamElements, un ticket est acheté dès que ton solde le permet, puis un autre à chaque fois que tu as assez de points, jusqu\'au maximum de tickets par personne. Le réglage reste d\'un giveaway à l\'autre ; il faut le compte StreamElements connecté (page Comptes).' }
    ]
  },
  {
    version: '0.47.0',
    date: '2026-11-03',
    title: 'Pseudo StreamElements et mises à jour automatiques',
    changes: [
      { kind: 'new', text: 'Fiche du streamer : « Pseudo StreamElements », à remplir quand sa page StreamElements n\'a pas son pseudo Twitch (par exemple streamelements.com/0riginalpiratematerial pour theoriginalpiratematerial). Ses points, ses giveaways StreamElements, le scanner et les liens utilisent alors la bonne page. Le pseudo ou l\'adresse de la page sont acceptés, et il est partagé avec tes amis.' },
      { kind: 'new', text: 'Paramètres → Mises à jour : « Installer automatiquement ». Une nouvelle version est installée dès qu\'elle est trouvée, sans cliquer, et l\'application redémarre seule. Le délai de vérification se règle en minutes, de 1 à 360.' }
    ]
  },
  {
    version: '0.46.0',
    date: '2026-11-03',
    title: 'Journal du chat par giveaway',
    changes: [
      { kind: 'new', text: 'Journal du chat découpé par giveaway : chaque section rappelle le giveaway (type, commande, cadeaux, gagnants, période) et regroupe les messages écrits pendant celui-ci. Un giveaway se termine à l\'annonce de son dernier gagnant, ou à sa clôture ; les messages d\'après vont au giveaway suivant de la chaîne.' },
      { kind: 'new', text: 'Page Alertes : le bouton Journal du chat d\'un giveaway ouvre le journal directement sur sa section, en cours comme passé (« Voir les plus récents » pour revenir aux derniers messages).' }
    ]
  },
  {
    version: '0.45.0',
    date: '2026-11-03',
    title: 'Mentions sur le téléphone',
    changes: [
      { kind: 'new', text: 'Web app du téléphone : section Mentions sur l\'accueil, sous les giveaways, avec les messages où un de tes pseudos est cité (qui, sur quelle chaîne, quand) et un bouton pour ouvrir le chat. Un appui sur une notification « Mention » ouvre directement cette section.' }
    ]
  },
  {
    version: '0.44.1',
    date: '2026-11-03',
    changes: [
      { kind: 'fixed', text: 'Gagnant annoncé par un bot modérateur avec une phrase courante (« 🎁 Le gagnant du tirage est … » écrit par WizeBot) : il est maintenant enregistré sur le giveaway, et plus seulement marqué « Gagnant » dans le journal du chat.' }
    ]
  },
  {
    version: '0.44.0',
    date: '2026-11-03',
    title: 'Scanner sur le téléphone',
    changes: [
      { kind: 'new', text: 'Web app du téléphone : nouvel onglet Scanner avec les streams repérés par le scan automatique (catégorie, spectateurs, titre, mots-clés trouvés, type de giveaway) et un bouton pour ouvrir le stream sur Twitch. Un appui sur une notification « Stream repéré » ouvre cet onglet.' },
      { kind: 'improved', text: 'Web app : l\'accueil indique quel giveaway est affiché, par exemple « En cours · 1 / 2 ». L\'onglet StreamElements s\'appelle « SE » dans la barre du bas pour que les 5 onglets tiennent.' }
    ]
  },
  {
    version: '0.43.0',
    date: '2026-11-03',
    title: 'Mise à jour du PC depuis le téléphone',
    changes: [
      { kind: 'new', text: 'Web app du téléphone : « Mise à jour du PC » dans le menu ⋮ en haut à droite. Elle affiche la version installée sur le PC, vérifie s\'il y en a une nouvelle et peut l\'installer : l\'application du PC se met à jour puis redémarre seule. Le PC doit être allumé avec l\'application ouverte.' }
    ]
  },
  {
    version: '0.42.0',
    date: '2026-11-03',
    title: 'Drops sur le téléphone',
    changes: [
      { kind: 'new', text: 'Web app du téléphone : nouvel onglet Drops, avec pour chaque compte la chaîne regardée, les campagnes en cours et la progression de chaque drop (minutes regardées, obtenu ou non).' },
      { kind: 'new', text: 'Notification « Drop obtenu » quand l\'application réclame un drop terminé : sur le PC, et sur le téléphone si tu l\'as choisie dans Notifications (menu ⋮ de la web app). Un appui ouvre l\'onglet Drops. Elle a aussi son alerte sonore, à activer dans les réglages des giveaways.' }
    ]
  },
  {
    version: '0.41.0',
    date: '2026-11-03',
    title: 'Points et StreamElements sur le téléphone',
    changes: [
      { kind: 'new', text: 'Web app du téléphone : onglets en bas de l\'écran avec deux nouvelles pages, Points de chaîne et StreamElements, pour chaque compte du PC (état, soldes, gains du jour, live, giveaway StreamElements en cours).' },
      { kind: 'new', text: 'Achat de tickets StreamElements depuis le téléphone : le PC fait l\'achat et le résultat s\'affiche sur le téléphone. À autoriser dans Paramètres → Synchronisation (désactivé par défaut).' },
      { kind: 'improved', text: 'Web app : les giveaways défilent comme des diapositives, un par écran, en glissant à gauche ou à droite.' },
      { kind: 'improved', text: 'Web app plus légère : une mise à jour des participations ou des soldes ne recharge plus les giveaways ni leurs images.' }
    ]
  },
  {
    version: '0.40.7',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Page Alertes : « Importer par lot (JSON) », sous les giveaways passés. Colle ou ouvre un fichier JSON, vérifie chaque giveaway (erreurs et cadeaux inconnus signalés), puis importe-les d\'un coup. Le format attendu est expliqué dans la fenêtre, avec un exemple ; un giveaway déjà présent n\'est pas importé deux fois.' }
    ]
  },
  {
    version: '0.40.6',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Page Alertes : « Ajouter un ancien giveaway », sous les giveaways passés, pour enregistrer après coup un giveaway déjà terminé avec sa date, ses cadeaux et ses gagnants. Il ne déclenche ni alerte ni participation, ne change pas la fiche du streamer, et il est partagé avec tes amis.' }
    ]
  },
  {
    version: '0.40.5',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Bouton « Actualiser » sur chaque tableau qui se met à jour tout seul : Points de chaîne, StreamElements, Chaînes, Mes gains, Drops (jeux souhaités) et Alertes (file d\'alertes, giveaways en cours, giveaways passés). Le statut live, les soldes et les giveaways sont relus tout de suite.' }
    ]
  },
  {
    version: '0.40.4',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Page Alertes : bouton « Filtres » à droite des giveaways passés, pour les filtrer par période, streamer, type, gagnant et cadeaux.' },
      { kind: 'new', text: 'Journal du chat : recherche par mots dans les messages, mots trouvés surlignés.' },
      { kind: 'new', text: 'Colonne Présence au tirage : une icône à côté signale qu\'une réponse automatique partira si tu gagnes ; au survol, le message et son délai.' },
      { kind: 'fixed', text: 'Drops : un jeu dont la campagne n\'est pas confirmée par Twitch (déjà terminée, ou détail illisible) n\'est plus regardé.' }
    ]
  },
  {
    version: '0.40.3',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Page Alertes : les giveaways passés se parcourent page par page (boutons à côté du titre), tous les giveaways terminés et plus seulement les plus récents.' },
      { kind: 'new', text: 'Fiche d\'un giveaway : la chance de gagner de chaque gagnant se saisit et se modifie à côté de son pseudo.' },
      { kind: 'improved', text: 'Page Alertes, giveaway StreamElements en cours : un clic sur ton pseudo ajoute d\'abord la chaîne à la page StreamElements de ce compte si elle n\'y est pas, puis l\'ouvre.' },
      { kind: 'fixed', text: 'La chance de gagner est enregistrée aussi quand le bot écrit « won the giveaway with a 20.92% chance », et pour chaque gagnant.' },
      { kind: 'fixed', text: 'Un giveaway StreamElements dont le lien est écrit avec des majuscules (« StreamElements.com ») s\'enregistre et se lit normalement.' }
    ]
  },
  {
    version: '0.40.1',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Pages Points de chaîne, StreamElements et Scanner : bouton « Regarder » juste avant la chaîne, pour ouvrir le stream en live dans la fenêtre des streams.' },
      { kind: 'improved', text: 'Photo de profil des chaînes aussi sur les pages Points de chaîne et StreamElements ; la cellule « Chaîne » est la même sur toutes les pages.' },
      { kind: 'improved', text: 'Page Alertes, giveaway StreamElements : chacun de tes comptes apparaît dans la colonne Participations, avec 0 ticket s\'il n\'en a pas.' }
    ]
  },
  {
    version: '0.40.0',
    date: '2026-11-03',
    title: 'Plusieurs gagnants par giveaway',
    changes: [
      { kind: 'new', text: 'Un giveaway peut avoir plusieurs gagnants : dans sa fiche, ajoute ou retire autant de gagnants que tu veux (tous retirés, il est rouvert).' },
      { kind: 'new', text: 'Journal du chat : désigner un gagnant sur un giveaway qui en a déjà un l\'ajoute à la liste au lieu de le remplacer.' },
      { kind: 'improved', text: 'Plusieurs annonces de gagnant à la suite dans le chat (dans les 15 minutes après le tirage) s\'ajoutent au même giveaway, sauf si un nouveau giveaway du streamer est arrivé entre-temps. Tous les gagnants d\'un giveaway StreamElements sont enregistrés.' },
      { kind: 'improved', text: 'Pages Alertes et StreamElements : tous les gagnants sont affichés, les tiens en vert ; « Mes gains » compte un giveaway dès qu\'un de tes comptes est parmi les gagnants.' },
      { kind: 'improved', text: 'Page StreamElements : le titre et la description du giveaway s\'affichent en entier, sans être coupés.' },
      { kind: 'improved', text: 'Fiche d\'un giveaway StreamElements : le lien du giveaway en premier, puis ce qu\'en dit StreamElements (titre, description, participants, tickets achetés, état), puis les tickets maximum, le tirage et la commande. Les gagnants et les notes, communs à tous les types, sont séparés en bas.' },
      { kind: 'fixed', text: 'Un giveaway StreamElements dont les participations sont fermées mais pas encore tiré reste ouvert dans l\'application, en attente du tirage.' }
    ]
  },
  {
    version: '0.39.5',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Page StreamElements : la colonne « Titre + description » affiche le titre du giveaway en gras, puis sa description.' },
      { kind: 'improved', text: 'Page Alertes : les cadeaux sont aussi repérés dans le titre et la description du giveaway StreamElements, et ajoutés au giveaway.' }
    ]
  },
  {
    version: '0.39.4',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Un nouveau giveaway reprend toutes les conditions de la fiche du streamer, y compris un giveaway détecté dans le chat. Quand tu changes les conditions d\'un giveaway en cours, la fiche du streamer est mise à jour pour ses prochains giveaways.' },
      { kind: 'improved', text: 'Dans le formulaire d\'un giveaway et la fiche du streamer, les conditions sont dans le même ordre que les colonnes des tableaux : présence au tirage, Discord, follow, bonjour, temps dans le chat.' }
    ]
  },
  {
    version: '0.39.3',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Fiche d\'un giveaway StreamElements : lien vers la page StreamElements du streamer, pour retrouver facilement le lien de son giveaway.' }
    ]
  },
  {
    version: '0.39.2',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Page Alertes, colonne Participations : sur un giveaway StreamElements terminé, un clic sur ton pseudo et tes tickets ouvre la page de ce giveaway sur StreamElements, dans ton navigateur. En cours, il ouvre toujours la page StreamElements de l\'application.' }
    ]
  },
  {
    version: '0.39.1',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Giveaway StreamElements : l\'application suit le giveaway StreamElements rattaché (par son lien) et le clôt dès que StreamElements désigne un gagnant, avec ce gagnant ; terminé sans gagnant, il est clos aussi. Le numéro du giveaway StreamElements est affiché dans sa fiche.' },
      { kind: 'fixed', text: 'Une annonce de gagnant du bot StreamElements pour un autre tirage ne clôt plus le giveaway StreamElements en cours.' }
    ]
  },
  {
    version: '0.39.0',
    date: '2026-11-03',
    title: 'Réponse automatique quand tu gagnes, et page Alertes plus claire',
    changes: [
      { kind: 'new', text: 'Réponse automatique quand un de tes comptes gagne un giveaway où la présence au tirage est requise : la phrase et le délai (10 s par défaut) se règlent dans la fiche du streamer, sous « Présence au tirage ». Une notification te laisse annuler la réponse avant son envoi.' },
      { kind: 'new', text: 'Paramètres → Changelog : la liste des nouveautés et des corrections de chaque version.' },
      { kind: 'new', text: 'Page StreamElements : colonne « Description » avec la description du giveaway en cours.' },
      { kind: 'improved', text: 'Page Alertes : les giveaways sont répartis en deux tableaux, « Giveaways en cours » et « Giveaways passés ».' },
      { kind: 'improved', text: 'Un giveaway terminé se rouvre avec le bouton « Rouvrir » en bas de sa fiche.' }
    ]
  },
  {
    version: '0.38.9',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Page Comptes : un message explique qu\'il faut avoir créé son compte sur streamelements.com avant de connecter StreamElements ici.' }
    ]
  },
  {
    version: '0.38.8',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Un clic sur la notification d\'un stream repéré par le scanner ouvre la page Scanner et met ce stream en surbrillance.' }
    ]
  },
  {
    version: '0.38.7',
    date: '2026-11-03',
    changes: [
      { kind: 'fixed', text: 'Drops : un jeu dont tu as déjà obtenu tous les drops n\'est plus regardé pour rien.' }
    ]
  },
  {
    version: '0.38.6',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Un giveaway terminé peut être rouvert depuis sa fiche.' },
      { kind: 'fixed', text: 'Les giveaways StreamElements et ceux ajoutés à la main ne sont plus terminés au bout de 12 h : ils passent en pause à la fin du live, même si l\'application était fermée à ce moment-là.' }
    ]
  },
  {
    version: '0.38.5',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Giveaway par points de chaîne : achat de la récompense directement depuis la page Alertes, avec les comptes de ton choix et une confirmation. Jamais d\'achat automatique, et rien n\'est acheté si le prix a changé.' }
    ]
  },
  {
    version: '0.38.4',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Fiche d\'un giveaway par points de chaîne : liste des récompenses de la chaîne (image, coût, en pause, épuisée) ; en choisir une remplit son nom et son coût.' }
    ]
  },
  {
    version: '0.38.3',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Photo de profil des chaînes sur les pages Alertes et Chaînes.' }
    ]
  },
  {
    version: '0.38.2',
    date: '2026-11-03',
    changes: [
      { kind: 'improved', text: 'Une notification Windows ouvre la bonne page même depuis le Centre de notifications, et lance l\'application si elle était fermée.' }
    ]
  },
  {
    version: '0.38.1',
    date: '2026-11-03',
    changes: [
      { kind: 'new', text: 'Un clic sur une notification Windows ouvre l\'application sur la page concernée (scanner, gains, giveaways).' }
    ]
  },
  {
    version: '0.38.0',
    date: '2026-11-02',
    title: 'Notifications sur le téléphone',
    changes: [
      { kind: 'new', text: 'La web app du téléphone reçoit les alertes en notifications, même fermée : giveaway détecté, gain, rappel de tirage, mention, scanner, au choix.' }
    ]
  },
  {
    version: '0.37.0',
    date: '2026-11-02',
    title: 'Les giveaways sur ton téléphone',
    changes: [
      { kind: 'new', text: 'Web app pour téléphone : giveaways en cours et en pause en temps réel, commande à copier, lots, conditions et participations de tes comptes. Connexion par QR code depuis Paramètres → Synchronisation.' }
    ]
  },
  {
    version: '0.36.0',
    date: '2026-11-02',
    changes: [
      { kind: 'improved', text: 'Synchronisation en temps réel : les modifications arrivent chez les autres en quelques secondes au lieu de 30 s.' }
    ]
  },
  {
    version: '0.35.1',
    date: '2026-11-02',
    changes: [
      { kind: 'improved', text: 'Page Drops : la campagne regardée est indiquée, et un bouton ouvre ton inventaire Twitch.' },
      { kind: 'fixed', text: 'Drops déjà obtenus : la chaîne n\'est plus regardée.' }
    ]
  },
  {
    version: '0.35.0',
    date: '2026-11-02',
    changes: [
      { kind: 'fixed', text: 'Page Drops : la liste des drops s\'affiche de nouveau pour toutes les campagnes, sans ralentir les comptes.' }
    ]
  },
  {
    version: '0.34.3',
    date: '2026-11-02',
    changes: [
      { kind: 'fixed', text: 'Un chat très actif ne fait plus perdre d\'événements (connexion, déconnexion) ni de commandes.' },
      { kind: 'improved', text: 'Les identifiants de secours sont chiffrés pour ta session Windows.' }
    ]
  },
  {
    version: '0.34.2',
    date: '2026-11-02',
    changes: [
      { kind: 'fixed', text: 'Scanner : les streams ignorés le restent après un redémarrage ou une mise à jour.' }
    ]
  },
  {
    version: '0.34.1',
    date: '2026-11-02',
    changes: [
      { kind: 'new', text: 'Un clic sur la vignette d\'un cadeau l\'affiche en grand, avec sa catégorie et son groupe.' },
      { kind: 'improved', text: 'Mise en page : fil d\'événements sur toute la hauteur, textes d\'aide sur toute la largeur.' }
    ]
  },
  {
    version: '0.34.0',
    date: '2026-11-02',
    title: 'Giveaways sur plusieurs lives',
    changes: [
      { kind: 'new', text: 'À la fin d\'un live, un giveaway dont le tirage est à venir passe « En pause » et reprend au live suivant.' },
      { kind: 'improved', text: 'Une commande vue de nouveau dans la semaine rouvre le giveaway au lieu d\'en créer un deuxième.' },
      { kind: 'improved', text: 'Les filtres et tris des pages sont retenus.' }
    ]
  },
  {
    version: '0.33.0',
    date: '2026-11-02',
    changes: [
      { kind: 'improved', text: 'Synchronisation plus légère : seuls les éléments modifiés sont envoyés.' }
    ]
  },
  {
    version: '0.32.2',
    date: '2026-11-02',
    changes: [
      { kind: 'new', text: 'Page StreamElements : colonnes « Giveaway » (en cours, tiré, clos) et « Gagnant ».' },
      { kind: 'fixed', text: 'Le gagnant annoncé par StreamElements est rattaché au bon giveaway.' }
    ]
  },
  {
    version: '0.32.0',
    date: '2026-11-02',
    changes: [
      { kind: 'improved', text: 'Interface plus légère et plus rapide.' },
      { kind: 'improved', text: 'Statut des lives partagé entre tes comptes : beaucoup moins de requêtes vers Twitch.' },
      { kind: 'new', text: 'Option (Paramètres → Journaux) : processeur, mémoire et trafic de l\'application, affichés en bas à gauche.' }
    ]
  },
  {
    version: '0.31.1',
    date: '2026-11-02',
    changes: [
      { kind: 'new', text: 'Le gagnant d\'un giveaway se modifie ou se retire dans sa fiche.' },
      { kind: 'fixed', text: 'Gagnant désigné depuis le journal du chat : il va au bon giveaway.' },
      { kind: 'fixed', text: 'Fenêtre des streams : les streams en pause reprennent bien quand on sort de la mise en avant.' }
    ]
  },
  {
    version: '0.31.0',
    date: '2026-11-02',
    title: 'Grand ménage',
    changes: [
      { kind: 'fixed', text: 'De nombreuses corrections : giveaways en double, commande envoyée deux fois, comptes déconnectés après une coupure réseau, chat qui ne revenait pas, erreurs qui ne s\'affichaient pas.' },
      { kind: 'improved', text: 'Sauvegarde de la base avant chaque mise à jour de sa structure.' }
    ]
  },
  {
    version: '0.30.0',
    date: '2026-11-01',
    changes: [
      { kind: 'fixed', text: 'Fenêtre des streams : la qualité choisie pour chaque stream est conservée après une mise à jour.' }
    ]
  },
  {
    version: '0.29.0',
    date: '2026-11-01',
    changes: [
      { kind: 'improved', text: 'Alertes sonores nettement plus fortes.' },
      { kind: 'new', text: 'Page Chaînes : colonnes des conditions (présence, Discord, follow, bonjour, temps dans le chat).' },
      { kind: 'improved', text: 'Bouton « Regarder » dans sa propre colonne, avant la chaîne.' }
    ]
  },
  {
    version: '0.28.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Détection des gagnants personnalisable : tes propres formulations et bots, avec un test de phrase.' },
      { kind: 'improved', text: 'Fiche du streamer modifiable depuis la page Chaînes et la fenêtre des streams.' },
      { kind: 'fixed', text: 'Après une mise à jour, l\'application revient affichée si elle l\'était.' }
    ]
  },
  {
    version: '0.27.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Dire bonjour dans le chat en un clic avec tous tes comptes, dans la langue de la chaîne.' }
    ]
  },
  {
    version: '0.26.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Jaquettes des jeux partout, et recherche des catégories avec jaquettes.' },
      { kind: 'improved', text: 'La fenêtre des streams se rouvre comme tu l\'avais laissée.' },
      { kind: 'improved', text: 'Les mentions restent dans la file d\'alertes après un redémarrage.' }
    ]
  },
  {
    version: '0.25.0',
    date: '2026-11-01',
    changes: [
      { kind: 'improved', text: 'Catégorie et nombre de spectateurs affichés sous le nom de la chaîne.' }
    ]
  },
  {
    version: '0.24.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Les réponses des bots à !giveaway sont affichées et servent à repérer les cadeaux.' },
      { kind: 'improved', text: 'Journal du chat plus complet (messages du streamer, filtres multiples).' }
    ]
  },
  {
    version: '0.23.0',
    date: '2026-11-01',
    changes: [
      { kind: 'fixed', text: 'Scanner : tous les lives de la catégorie sont lus, petits streamers compris.' }
    ]
  },
  {
    version: '0.22.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Option pour bloquer les publicités Twitch dans la fenêtre des streams.' }
    ]
  },
  {
    version: '0.21.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Gagnants annoncés par les modérateurs ou le streamer reconnus.' },
      { kind: 'new', text: 'Journal du chat : recherche par auteur, gagnant désigné à la main.' },
      { kind: 'improved', text: 'Une commande tapée par toi dans le chat compte comme participation.' }
    ]
  },
  {
    version: '0.20.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Nouvelles conditions de giveaway, dont le follow depuis un certain nombre de jours.' },
      { kind: 'new', text: 'Fenêtre des streams connectée à ta session Twitch (Windows).' }
    ]
  },
  {
    version: '0.19.0',
    date: '2026-11-01',
    changes: [
      { kind: 'improved', text: 'La page multi-stream fait maintenant partie de l\'application.' }
    ]
  },
  {
    version: '0.17.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Journal du chat des chaînes surveillées, gardé 14 jours.' },
      { kind: 'new', text: 'Détection des mots-clés tapés sans « ! » et des annonces de gagnant de tous les bots.' }
    ]
  },
  {
    version: '0.16.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Cadeaux favoris.' },
      { kind: 'new', text: 'Follow et invitation Discord en un clic depuis la page Alertes.' }
    ]
  },
  {
    version: '0.15.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Réglages de la détection des giveaways (seuil, messages analysés…).' },
      { kind: 'improved', text: 'Commandes ignorées enregistrées dans la fiche du streamer.' }
    ]
  },
  {
    version: '0.14.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Choix du son de chaque alerte, ou ton propre fichier son.' },
      { kind: 'fixed', text: 'La fenêtre des streams ne bloque plus l\'application sous Windows.' }
    ]
  },
  {
    version: '0.13.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Fenêtre multi-stream pour regarder plusieurs lives à la fois.' }
    ]
  },
  {
    version: '0.12.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Points de chaîne : solde maximum par chaîne.' },
      { kind: 'new', text: 'Paramètres en sections, et lecture des journaux.' }
    ]
  },
  {
    version: '0.11.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Alertes sonores.' },
      { kind: 'new', text: 'Filtres des chaînes surveillées par catégorie et par cadeau.' }
    ]
  },
  {
    version: '0.10.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Synchronisation entre utilisateurs : cadeaux, giveaways et fiches des streamers mis en commun.' }
    ]
  },
  {
    version: '0.9.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Détection des cadeaux dans les titres, descriptions et messages épinglés.' }
    ]
  },
  {
    version: '0.8.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Scanner : repère les lives qui parlent de giveaway dans tes catégories.' },
      { kind: 'new', text: 'Groupes de cadeaux.' }
    ]
  },
  {
    version: '0.7.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Catalogue de cadeaux, page « Mes gains » et fiche du streamer.' }
    ]
  },
  {
    version: '0.6.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Fiche complète des giveaways (types, conditions, heure du tirage) et rappel avant le tirage.' }
    ]
  },
  {
    version: '0.5.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Giveaways : détection dans le chat, alertes, participation en un clic.' }
    ]
  },
  {
    version: '0.4.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Drops Twitch récupérés automatiquement.' }
    ]
  },
  {
    version: '0.3.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Compte StreamElements : tickets exacts et achat de tickets.' }
    ]
  },
  {
    version: '0.2.0',
    date: '2026-11-01',
    changes: [
      { kind: 'new', text: 'Points StreamElements gagnés en restant présent dans le chat.' }
    ]
  },
  {
    version: '0.1.0',
    date: '2026-11-01',
    title: 'Première version',
    changes: [
      { kind: 'new', text: 'Points de chaîne récupérés sur plusieurs comptes, et mises à jour automatiques.' }
    ]
  }
]
