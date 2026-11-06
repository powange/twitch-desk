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
    version: '0.48.56',
    date: '2026-11-06',
    title: 'Web app : participer et agir à distance',
    changes: [
      { kind: 'new', text: 'Web app, giveaways : bouton « Participer avec … » (ou « Acheter avec … » pour un giveaway par points de chaîne, si les achats depuis le téléphone sont autorisés). Le PC envoie la commande avec tes comptes qui n\'y sont pas encore.' },
      { kind: 'new', text: 'Web app, giveaways : raccourcis des conditions, « Suivre la chaîne » (follow requis), « Dire bonjour » (bonjour requis) et « Discord » (lien de la fiche du streamer).' },
      { kind: 'new', text: 'Web app : quand un de tes comptes gagne, un bandeau affiche la réponse prévue avec un compte à rebours et un bouton « Annuler ».' },
      { kind: 'new', text: 'Web app, Points de chaîne : interrupteurs « Récolte » et « Bonus auto », et solde maximum de chaque chaîne.' },
      { kind: 'new', text: 'Web app, Drops : ignorer une campagne (ou la rétablir) et réclamer les drops terminés.' },
      { kind: 'new', text: 'Web app : mettre l\'application du PC en pause ou la relancer, depuis le menu. Un bandeau signale la pause.' }
    ]
  },
  {
    version: '0.48.55',
    date: '2026-11-06',
    title: 'Web app plus complète',
    changes: [
      { kind: 'new', text: 'Web app, Points de chaîne : l\'ordre des chaînes se change en faisant glisser la poignée de chaque ligne, comme sur le PC.' },
      { kind: 'new', text: 'Web app, StreamElements : interrupteurs « Chat » (présence dans le chat) et « Achat auto » sur chaque chaîne. Activer l\'achat automatique demande que les achats depuis le téléphone soient autorisés sur le PC.' },
      { kind: 'new', text: 'Web app, giveaways : coche « Afficher les giveaways en pause » (live terminé, tirage pas encore fait), décochée au départ.' },
      { kind: 'improved', text: 'Web app : drapeau de la langue du streamer, comme sur le PC, à côté de son nom sur toutes les pages, et sa photo de profil sur les giveaways, les mentions et les drops.' },
      { kind: 'fixed', text: 'Scanner (PC et web app) : une chaîne qu\'on vient de surveiller est marquée « Surveillée » tout de suite, sans attendre le scan suivant. Après une demande faite depuis le téléphone, le PC lui renvoie aussitôt son état à jour.' }
    ]
  },
  {
    version: '0.48.54',
    date: '2026-11-06',
    title: 'Web app : masquer, journal, conditions et photos',
    changes: [
      { kind: 'new', text: 'Web app du téléphone, giveaways : boutons « Masquer » (ou « Afficher »), « Journal » (journal du chat pendant le giveaway) et « Conditions » (les modifier ; un giveaway en cours met aussi à jour la fiche du streamer, comme sur le PC). Une option « Afficher les masqués » fait réapparaître les giveaways masqués. Le PC exécute chaque demande.' },
      { kind: 'new', text: 'Web app, Scanner : bouton « Surveiller » pour ajouter un stream repéré aux chaînes surveillées du PC. Les lignes trop longues passent à la ligne au lieu d\'être coupées.' },
      { kind: 'fixed', text: 'Web app : les photos des cadeaux hébergées ailleurs que chez Twitch s\'affichent maintenant. Le serveur de synchronisation (version 0.8.10) va les chercher lui-même, l\'adresse du téléphone n\'est donc jamais connue de leur site.' }
    ]
  },
  {
    version: '0.48.53',
    date: '2026-11-06',
    title: 'Solde StreamElements sans présence dans le chat',
    changes: [
      { kind: 'fixed', text: 'Page StreamElements : une chaîne dont la présence dans le chat est désactivée voit maintenant son solde relu toutes les 30 minutes pendant son live. Avant, il n\'était plus relu du tout, alors qu\'il peut changer (tickets achetés, points donnés par le streamer, récolte sur un autre PC).' }
    ]
  },
  {
    version: '0.48.52',
    date: '2026-11-06',
    title: 'Journal du chat corrigé pour les bots',
    changes: [
      { kind: 'fixed', text: 'Journal du chat : quand un bot est reconnu à son badge « Bot de chat », ses anciens messages notés à tort comme commandes de spectateur passent en messages de bot (par exemple les « Catch it using !pokecatch » de PokemonCommunityGame).' }
    ]
  },
  {
    version: '0.48.51',
    date: '2026-11-06',
    title: 'Réponse au gain en plusieurs phrases',
    changes: [
      { kind: 'improved', text: 'Réponse automatique si tu gagnes : on peut maintenant écrire plusieurs phrases, une par ligne (5 au plus). Elles sont envoyées l\'une après l\'autre : la première après le délai habituel, les suivantes espacées d\'un délai réglable (3 secondes par défaut). La réponse reste annulable jusqu\'à la dernière phrase.' }
    ]
  },
  {
    version: '0.48.50',
    date: '2026-11-06',
    title: 'Giveaways passés dans l\'ordre, heure du tirage',
    changes: [
      { kind: 'fixed', text: 'Page Alertes, giveaways passés : ils sont rangés par date de détection, la plus récente en premier, comme l\'indique la colonne « Détecté ». Corriger le gagnant d\'un ancien giveaway ne le fait plus remonter en haut de la liste. Le filtre par période utilise la même date.' },
      { kind: 'fixed', text: 'Giveaways : quand un gagnant est détecté (annonce d\'un bot ou tirage StreamElements), l\'heure du tirage est remplie si elle n\'était pas connue. C\'est l\'heure du tirage donnée par StreamElements, ou celle de l\'annonce dans le chat.' }
    ]
  },
  {
    version: '0.48.49',
    date: '2026-11-06',
    title: 'Bots de chat reconnus à leur badge',
    changes: [
      { kind: 'improved', text: 'Giveaways : un bot qui porte le badge « Bot de chat » de Twitch est reconnu comme un bot, quel que soit son pseudo. Ses annonces de gagnant sont lues comme celles de StreamElements ou Nightbot.' },
      { kind: 'fixed', text: 'Chat : les messages écrits en « /me » (texte en italique, souvent utilisé par les bots) sont maintenant lus correctement par la détection.' }
    ]
  },
  {
    version: '0.48.48',
    date: '2026-11-06',
    title: 'Participation automatique depuis un giveaway',
    changes: [
      { kind: 'improved', text: 'Modifier un giveaway : la section « Participation automatique » de la fiche du streamer est aussi dans le menu. Les commandes ajoutées ou changées là sont enregistrées dans la fiche, avec le giveaway.' }
    ]
  },
  {
    version: '0.48.47',
    date: '2026-11-06',
    title: 'Participation automatique hors giveaway',
    changes: [
      { kind: 'new', text: 'Fiche d\'un streamer : nouvelle section « Participation automatique ». On y liste les commandes que le streamer propose de taper un court moment pour gagner des points, hors giveaway. Quand plusieurs spectateurs en tapent une, l\'application la tape avec tes comptes après quelques secondes, puis attend 10 minutes avant de recommencer. Une commande peut aussi rester dans les commandes ignorées, pour ne pas déclencher d\'alerte de giveaway.' },
      { kind: 'new', text: 'La liste de ces commandes est partagée entre tes PC, mais y répondre se choisit commande par commande sur chaque PC. Une commande ajoutée sur un autre PC arrive désactivée.' }
    ]
  },
  {
    version: '0.48.46',
    date: '2026-11-06',
    title: 'Giveaway en cours sur chaque stream',
    changes: [
      { kind: 'improved', text: 'Fenêtre des streams : dès qu\'un giveaway est en cours sur un stream affiché, son type (Commande, Ticket, StreamElements, Points de chaîne) et sa commande s\'affichent dans l\'en-tête du stream, même si tu y participes déjà. Un clic sur la commande la copie. « Participer » ne s\'ajoute que s\'il reste des comptes à inscrire.' }
    ]
  },
  {
    version: '0.48.45',
    date: '2026-11-06',
    title: 'Participer depuis la fenêtre des streams',
    changes: [
      { kind: 'new', text: 'Fenêtre des streams : quand un giveaway ouvert attend ta participation sur un stream affiché, un bouton « Participer » apparaît à droite du pseudo, dans l\'en-tête du stream, avec la commande à côté. « Participer » envoie la commande avec tes comptes qui n\'y sont pas encore. Un clic sur la commande la copie dans le presse-papiers.' }
    ]
  },
  {
    version: '0.48.44',
    date: '2026-11-05',
    title: 'Type et commande réunis',
    changes: [
      { kind: 'improved', text: 'Page Alertes : les colonnes Type et Commande n\'en font plus qu\'une. Le type s\'affiche, avec la commande juste en dessous (un clic la copie, le crayon la modifie).' },
      { kind: 'improved', text: 'Page Alertes : un giveaway StreamElements qui a son lien affiche le type « StreamElements » avec une icône de lien externe. Un clic ouvre le giveaway sur StreamElements, dans le navigateur.' }
    ]
  },
  {
    version: '0.48.43',
    date: '2026-11-05',
    title: 'Menu d\'actions des giveaways',
    changes: [
      { kind: 'improved', text: 'Page Alertes : les actions d\'un giveaway (journal du chat, modifier, masquer, clore, supprimer) sont regroupées dans un menu « ⋯ » au bout de la ligne, et s\'ouvrent aussi d\'un clic droit n\'importe où sur la ligne. Participer et Acheter restent des boutons visibles.' },
      { kind: 'improved', text: 'Pages Alertes et Chaînes : le bloc des colonnes de conditions est encadré d\'un trait vertical à gauche et à droite, pour mieux le distinguer des colonnes voisines.' }
    ]
  },
  {
    version: '0.48.42',
    date: '2026-11-05',
    title: 'Connexion temps réel stable',
    changes: [
      { kind: 'fixed', text: 'Temps réel : les débuts et fins de live, les spectateurs et les raids passent aussi par la nouvelle connexion de Twitch. L\'ancienne coupait toutes les quelques secondes, avec des erreurs dans le journal. Elle ne sert plus qu\'en secours.' }
    ]
  },
  {
    version: '0.48.41',
    date: '2026-11-05',
    title: 'Points de chaîne : connexion temps réel rétablie',
    changes: [
      { kind: 'fixed', text: 'Points de chaîne : depuis le 4 novembre, Twitch coupait la connexion temps réel qui annonce les points gagnés, les bonus à cliquer et la progression des drops. Les gains du jour restaient à 0 et le statut « temps réel » ne tenait pas. L\'application utilise maintenant la même connexion que le site de Twitch.' }
    ]
  },
  {
    version: '0.48.40',
    date: '2026-11-05',
    title: 'Ligne survolée surlignée',
    changes: [
      { kind: 'improved', text: 'Tableaux : la ligne sous la souris est surlignée sur toute sa largeur, pour suivre la même ligne d\'un bout à l\'autre (aussi sur les lignes déjà colorées et les colonnes figées).' }
    ]
  },
  {
    version: '0.48.39',
    date: '2026-11-05',
    title: 'Mise à jour installée au démarrage',
    changes: [
      { kind: 'improved', text: 'Au lancement de l\'application, une nouvelle version trouvée est installée tout de suite, sans cliquer : l\'application redémarre seule à jour. Pendant qu\'elle tourne, rien ne change : la mise à jour attend ta confirmation, sauf si « Installer automatiquement » est coché. Il suffit de désactiver « Chercher les mises à jour automatiquement » pour garder la main.' }
    ]
  },
  {
    version: '0.48.38',
    date: '2026-11-05',
    title: 'Conditions reprises de la fiche',
    changes: [
      { kind: 'new', text: 'Modifier un giveaway, même clos, section Conditions : si la fiche du streamer a d\'autres conditions, un bouton « Reprendre celles de la fiche » les recopie toutes d\'un coup (celles que la fiche ne connaît pas restent comme elles sont).' },
      { kind: 'fixed', text: 'Drops : plus de notification pour les drops d\'une campagne ignorée. Ils sont toujours réclamés et notés dans le journal.' }
    ]
  },
  {
    version: '0.48.37',
    date: '2026-11-05',
    title: 'Heure du tirage facultative',
    changes: [
      { kind: 'improved', text: 'Formulaire d\'un giveaway : le tirage se saisit en deux champs, le jour et l\'heure. L\'heure est facultative : sans heure, le tirage compte pour toute la journée choisie (jusqu\'à 23 h 59), et seul le jour est affiché (pas de rappel « reste présent » dans ce cas).' }
    ]
  },
  {
    version: '0.48.36',
    date: '2026-11-05',
    title: 'Gain offrable à quelqu\'un d\'autre',
    changes: [
      { kind: 'new', text: 'Conditions d\'un giveaway et fiche d\'un streamer : nouvelle information « Gain offrable à quelqu\'un d\'autre » (Oui, Non ou ?). Celle de la fiche pré-remplit ses nouveaux giveaways, et la changer sur un giveaway met la fiche à jour.' },
      { kind: 'new', text: 'Pages Chaînes et Alertes : nouvelle colonne pour cette information dans les tableaux des conditions. Elle apparaît aussi dans la web app du téléphone et se synchronise entre tes PC.' }
    ]
  },
  {
    version: '0.48.35',
    date: '2026-11-05',
    title: 'Adresse de la web app',
    changes: [
      { kind: 'new', text: 'Paramètres, Synchronisation : l\'adresse de la web app du téléphone s\'affiche, avec des boutons pour la copier et l\'ouvrir.' },
      { kind: 'improved', text: 'L\'adresse de la web app peut être réglée une fois pour toutes sur le serveur de synchronisation (version 0.8.8 et plus) : tous les PC l\'utilisent, pour l\'affichage comme pour le QR code de connexion d\'un téléphone, sans rien régler. Le champ du PC reste disponible pour utiliser une autre adresse sur ce poste.' }
    ]
  },
  {
    version: '0.48.34',
    date: '2026-11-05',
    title: 'Réponse au gain plus fiable',
    changes: [
      { kind: 'fixed', text: 'Réponse automatique si tu gagnes : elle part aussi quand la présence au tirage n\'est pas indiquée sur le giveaway (« ? ») mais l\'est dans la fiche du streamer, par exemple pour un giveaway détecté avant que tu remplisses la fiche.' },
      { kind: 'fixed', text: 'Elle part aussi quand le gain est annoncé dans le chat sur une chaîne où aucun giveaway n\'a été trouvé, si la fiche du streamer demande la présence.' },
      { kind: 'fixed', text: 'Giveaway StreamElements : le délai compte depuis l\'annonce du bot dans le chat. La réponse n\'est plus perdue quand StreamElements est relu en retard (erreur réseau) ou que l\'horloge du PC est décalée.' },
      { kind: 'fixed', text: 'Si StreamElements a donné le gagnant juste avant l\'annonce du bot, sans réponse possible à ce moment-là, l\'annonce la déclenche quand même (une seule fois par gain).' }
    ]
  },
  {
    version: '0.48.33',
    date: '2026-11-05',
    title: 'Mêmes règles partout',
    changes: [
      { kind: 'fixed', text: 'Accents : le rattachement automatique des cadeaux et le scanner retirent tous les accents, comme l\'affichage (« Wiedźmin », « São », accent tapé à part sur Mac ou iPhone). Un cadeau affiché comme repéré est donc bien rattaché.' },
      { kind: 'improved', text: 'Cadeaux repérés : mêmes textes partout et pour le rattachement automatique (titre du live, description, message épinglé, giveaway StreamElements en cours, réponses des bots), seulement pendant le live. La page StreamElements y ajoute le giveaway relevé par les points.' },
      { kind: 'fixed', text: 'Synchronisation : les cadeaux d\'un giveaway sont fusionnés entre les PC au lieu d\'être remplacés par ceux du dernier PC, et un cadeau retiré à la main sur un PC l\'est aussi sur les autres.' },
      { kind: 'improved', text: 'Mise en avant (bordure violette) : une seule règle, un cadeau favori rattaché ou repéré, sur les pages Chaînes, Alertes, StreamElements et Scanner.' },
      { kind: 'improved', text: 'Mots-clés des cadeaux nettoyés pareil dans le formulaire, l\'import et la synchronisation : doublons sans tenir compte des accents (« épée » et « epee »), mot-clé identique au titre retiré, longueur et nombre limités.' },
      { kind: 'fixed', text: 'StreamElements : le lien posté par le bot et le giveaway en cours trouvé sur la page suivent la même règle. Un giveaway clos à la main n\'est plus jamais rouvert tout seul (commande reprise dans le chat, lien reposté), un giveaway clos automatiquement qui tourne encore est rouvert, et rien n\'est créé hors des catégories suivies.' },
      { kind: 'fixed', text: 'StreamElements : la commande et le maximum de tickets d\'un giveaway créé depuis le lien du bot sont complétés à la relecture suivante.' },
      { kind: 'fixed', text: 'Achat automatique de tickets : rien n\'est acheté pour un giveaway clos à la main dans l\'application, ni après une lecture StreamElements en échec.' },
      { kind: 'fixed', text: 'Identifiant StreamElements d\'une chaîne : un seul cache pour toute l\'application, revérifié chaque jour, et oublié partout (points, analyses, scanner) quand le pseudo StreamElements change par la synchronisation.' },
      { kind: 'fixed', text: 'Formulaire : le giveaway StreamElements en cours ne remplace plus la commande ou le maximum que tu as tapés pendant le chargement.' },
      { kind: 'improved', text: 'Spectateurs arrondis aussi dans l\'état envoyé au téléphone : moins d\'envois au serveur.' },
      { kind: 'fixed', text: 'Tableau de bord : nombre de bonus du jour exact même en fin de journée chargée, et une chaîne suivie par deux comptes n\'est comptée qu\'une fois dans « regardées / en live ».' },
      { kind: 'fixed', text: 'Scanner : les mots-clés trouvés sont surlignés même avec un accent ou un tiret (« CLÉ », « FREE-KEYS »).' },
      { kind: 'fixed', text: 'Drops : « À réclamer » s\'affiche exactement quand l\'application va réclamer le drop (sur le PC et le téléphone).' }
    ]
  },
  {
    version: '0.48.32',
    date: '2026-11-05',
    title: 'Application plus légère',
    changes: [
      { kind: 'improved', text: 'Moins de travail en arrière-plan : l\'état d\'un compte n\'est plus renvoyé à l\'interface ni au serveur toutes les 2 secondes quand rien n\'a changé, et les campagnes de drops ne repartent plus à chaque changement du nombre de spectateurs.' },
      { kind: 'improved', text: 'Le repérage des cadeaux ne relit plus les images de tout le catalogue à chaque passage, et chaque message du chat est analysé une seule fois.' },
      { kind: 'improved', text: 'StreamElements : le giveaway en cours d\'une chaîne est lu une seule fois pour toute l\'application (points, giveaways, analyses, scanner, formulaire), son détail n\'est plus relu tant qu\'il est en cours, et plusieurs gagnants annoncés d\'affilée ne lancent plus une vérification chacun.' },
      { kind: 'improved', text: 'Créer ou modifier un giveaway ne recharge plus tous les réglages ni les connexions ; les cadeaux des tableaux (Alertes, Chaînes, StreamElements, Scanner) sont calculés une fois par mise à jour avec le même affichage partout.' }
    ]
  },
  {
    version: '0.48.31',
    date: '2026-11-05',
    title: 'Corrections de logique (audit)',
    changes: [
      { kind: 'fixed', text: 'Réponse automatique au gain : revérifiée juste avant l\'envoi. Si tu retires le gagnant, masques le giveaway ou mets l\'application en pause pendant le délai, elle ne part plus.' },
      { kind: 'fixed', text: 'Un giveaway sur plusieurs lives (saisi à la main, StreamElements) finit bien par expirer : la limite de 14 jours compte depuis sa première détection, plus depuis la dernière reprise du live.' },
      { kind: 'fixed', text: 'Plus de double « Tu as gagné ! » quand le gain est annoncé deux fois (bot puis modérateur) sur une chaîne sans giveaway trouvé.' },
      { kind: 'fixed', text: 'Un cadeau que tu retires d\'un giveaway n\'y est plus jamais remis automatiquement (titre du live, réponses des bots, StreamElements). Un cadeau rattaché pendant que le formulaire est ouvert n\'est plus effacé à l\'enregistrement.' },
      { kind: 'fixed', text: 'Mots-clés et noms de cadeau de moins de 2 lettres ou chiffres (« a », « + ») ignorés par le repérage, et refusés dans le formulaire du cadeau : ils rattachaient le cadeau à presque tous les giveaways.' },
      { kind: 'fixed', text: 'Un même giveaway StreamElements n\'est plus créé deux fois quand son lien porte le pseudo Twitch d\'un côté et le vrai nom de la page StreamElements de l\'autre. Le formulaire ne propose plus un giveaway StreamElements déjà suivi, et refuse de le recréer.' },
      { kind: 'fixed', text: 'Achat automatique de tickets StreamElements : arrêté pour un giveaway réservé aux abonnés après le premier refus, au lieu de réessayer toutes les 5 minutes.' },
      { kind: 'fixed', text: 'Une erreur passagère de Twitch ou StreamElements n\'efface plus la description, le message épinglé et le giveaway StreamElements affichés pour une chaîne.' },
      { kind: 'fixed', text: 'Drops : les drops terminés d\'une campagne ignorée sont quand même réclamés (ignorer arrête seulement le visionnage).' },
      { kind: 'fixed', text: 'Statut live : avec plus de 50 chaînes, un lot en échec ne fait plus perdre les autres.' }
    ]
  },
  {
    version: '0.48.30',
    date: '2026-11-05',
    title: 'Cadeaux au nom commençant par un symbole',
    changes: [
      { kind: 'fixed', text: 'Un cadeau dont le nom ou un mot-clé commence par un symbole (« €-CARD ») est repéré même collé à un chiffre ou une lettre (« 20€-CARD » dans le titre du live).' }
    ]
  },
  {
    version: '0.48.29',
    date: '2026-11-05',
    title: 'Cadeaux du live rattachés tout seuls',
    changes: [
      { kind: 'new', text: 'Un cadeau repéré dans le titre du live, la description ou le message épinglé est maintenant rattaché tout seul aux giveaways ouverts de la chaîne, comme si tu l\'avais ajouté à la main (il reste après la fin du live, et sur les autres appareils). C\'était déjà le cas pour les réponses des bots et le giveaway StreamElements.' },
      { kind: 'improved', text: 'Un cadeau retiré à la main n\'est pas remis tant que le titre, la description, le message épinglé et le catalogue de cadeaux ne changent pas.' },
      { kind: 'fixed', text: 'Un giveaway tout juste détecté reçoit bien les cadeaux des réponses de bot déjà connues sur la chaîne.' }
    ]
  },
  {
    version: '0.48.28',
    date: '2026-11-05',
    title: 'Cadeaux repérés sur les giveaways en cours',
    changes: [
      { kind: 'improved', text: 'Giveaways en cours, colonne Cadeaux : en plus des cadeaux rattachés, ceux repérés dans le titre du live, la description, le message épinglé, le giveaway StreamElements et les réponses des bots s\'affichent (marqués d\'une petite étincelle), comme sur la page Chaînes. Un cadeau favori repéré met aussi la ligne en avant.' }
    ]
  },
  {
    version: '0.48.27',
    date: '2026-11-05',
    title: 'Icône pour les cadeaux sans image',
    changes: [
      { kind: 'improved', text: 'Un cadeau sans image s\'affiche avec une icône de cadeau partout, y compris sur la page Gains, où rien ne s\'affichait.' },
      { kind: 'fixed', text: 'Une image de cadeau qui ne se charge plus (adresse disparue) est remplacée par l\'icône de cadeau au lieu d\'une image cassée, sur le PC comme sur le téléphone.' }
    ]
  },
  {
    version: '0.48.26',
    date: '2026-11-04',
    title: 'Masquer un giveaway en cours',
    changes: [
      { kind: 'new', text: 'Giveaways en cours : bouton « Masquer » (œil barré) sur chaque giveaway. Masqué, il disparaît de la liste, ne donne plus d\'alerte (carte, notification du PC et du téléphone), et ni la participation automatique ni l\'achat automatique de tickets StreamElements ne s\'en occupent. Seul ce giveaway est concerné : le streamer reste suivi.' },
      { kind: 'new', text: 'Case « Afficher les masqués » au-dessus des giveaways en cours, pour les revoir (grisés) et les remettre avec « Ne plus masquer ». Le masquage reste propre à ce PC.' }
    ]
  },
  {
    version: '0.48.25',
    date: '2026-11-04',
    title: 'Message épinglé gardé, cadeaux sur la page StreamElements',
    changes: [
      { kind: 'new', text: 'Journal du chat : la dernière version du message épinglé est gardée. Streamer hors ligne ou message retiré : elle reste affichée, avec la date à laquelle elle a été vue. Le message est aussi relu chaque minute tant que le journal est ouvert.' },
      { kind: 'new', text: 'Page StreamElements : colonne Cadeaux, avec les cadeaux rattachés au giveaway de la chaîne, puis ceux repérés dans le titre et la description du giveaway StreamElements en cours (petite étincelle), favoris étoilés.' }
    ]
  },
  {
    version: '0.48.24',
    date: '2026-11-04',
    title: 'Message épinglé dans le journal du chat',
    changes: [
      { kind: 'new', text: 'Journal du chat : le message épinglé du chat de la chaîne est affiché tout en haut.' },
      { kind: 'improved', text: 'Journal du chat : quand un bot répond de nouveau à une commande (!giveaway…) pendant que le journal est ouvert, sa dernière réponse remplace l\'ancienne tout de suite.' }
    ]
  },
  {
    version: '0.48.23',
    date: '2026-11-04',
    title: 'Lien StreamElements trouvé tout seul, favoris sur la page Alertes',
    changes: [
      { kind: 'new', text: 'Formulaire de giveaway : avec le type StreamElements et sans lien, le giveaway en cours sur la page StreamElements du streamer est trouvé et son lien rempli tout seul (avec la commande et le maximum de tickets s\'ils sont vides). Un lien déjà saisi n\'est jamais remplacé.' },
      { kind: 'new', text: 'Page Alertes : petite étoile sur les cadeaux favoris, et ligne mise en avant par une bordure violette quand un giveaway (en cours ou passé) a un cadeau favori, comme sur la page Chaînes.' }
    ]
  },
  {
    version: '0.48.22',
    date: '2026-11-04',
    title: 'Performances',
    changes: [
      { kind: 'improved', text: 'Web app : la vue du téléphone se charge bien plus vite et consomme beaucoup moins de données mobiles (images des lots chargées à part et gardées en mémoire, seuls les giveaways en cours sont lus). Le téléphone ne relit plus tout quand une chaîne ou une fiche change, et son cache ne grossit plus à chaque mise à jour.' },
      { kind: 'improved', text: 'Un changement du nombre de spectateurs ne relance plus de synchronisation, et les petites actions (lot reçu, gagnants corrigés, ancien giveaway ajouté) ne rechargent plus tout le suivi des giveaways.' },
      { kind: 'improved', text: 'Giveaways StreamElements en pause ou d\'une chaîne hors ligne : vérifiés toutes les 45 minutes au lieu de 3 (et tout de suite au retour en live).' },
      { kind: 'improved', text: 'La comparaison complète de la synchronisation ne bloque plus le chat, les drops et les points pendant qu\'elle tourne, et la liste des giveaways se lit plus vite.' },
      { kind: 'improved', text: 'Journal du chat, page StreamElements et grands tableaux plus fluides (surlignage calculé une fois, photos de profil chargées au défilement).' }
    ]
  },
  {
    version: '0.48.21',
    date: '2026-11-04',
    title: 'Frictions du rapport (gravité basse)',
    changes: [
      { kind: 'improved', text: 'Scanner : une notification d\'un stream ignoré l\'affiche le temps de la visite, sans cocher « Afficher les ignorés » pour la suite.' },
      { kind: 'improved', text: 'Réponse automatique : l\'annonce ne se ferme plus par erreur pendant le délai, et « Annuler » prévient si la réponse est déjà partie (« Trop tard »).' },
      { kind: 'improved', text: 'Import par lot : texte vidé après un import, avertissement si le texte a changé depuis la vérification, message si le fichier ne peut pas être lu.' },
      { kind: 'improved', text: 'Filtres des giveaways passés : « Du » et « Au » ne peuvent plus être inversés, et un message le signale.' },
      { kind: 'fixed', text: 'Fenêtre « Acheter la récompense ? » : plus de texte vide pendant sa fermeture.' },
      { kind: 'improved', text: 'Formulaire de giveaway dans une fenêtre étroite : sections en onglets au-dessus, types sur deux lignes.' },
      { kind: 'improved', text: 'Web app : les notifications continuent quand le navigateur renouvelle son abonnement, la web app s\'affiche (avec sa dernière vue datée) quand le serveur redémarre ou ne répond pas.' },
      { kind: 'improved', text: 'Serveur : un code d\'association n\'est plus perdu quand la limite de 20 téléphones est atteinte ; avertissements et documentation pour le contact des notifications et le réglage derrière Cloudflare.' }
    ]
  },
  {
    version: '0.48.20',
    date: '2026-11-04',
    title: 'Frictions du rapport (gravité moyenne)',
    changes: [
      { kind: 'fixed', text: 'StreamElements : une chaîne vue « sans StreamElements » est revérifiée au lancement, sur « Actualiser » et chaque jour (elle a pu s\'inscrire depuis), au lieu de ne plus jamais l\'être.' },
      { kind: 'fixed', text: 'Import par lot : deux giveaways du même jour sur la même chaîne (sans heure) sont tous les deux importés s\'ils diffèrent (type, commande, lien ou gagnant).' },
      { kind: 'fixed', text: 'Synchronisation : un code refusé par le serveur n\'est plus réessayé toutes les 30 secondes (nouvel essai au bout de 30 minutes, ou tout de suite avec « Synchroniser »). Le serveur ne bloque plus les téléphones de la maison à cause des erreurs de code d\'un PC.' },
      { kind: 'improved', text: 'Web app sur iPhone : l\'écran de connexion conseille d\'ajouter d\'abord l\'icône à l\'écran d\'accueil et de se connecter depuis elle (une connexion faite dans Safari n\'y est pas reprise).' }
    ]
  },
  {
    version: '0.48.19',
    date: '2026-11-04',
    title: 'Corrections des bugs mineurs du rapport',
    changes: [
      { kind: 'fixed', text: 'Maximum de tickets annoncé par StreamElements (« got N tickets (N/MAX) ») : noté sur le giveaway StreamElements (ou par tickets) de la chaîne, plus jamais sur un giveaway par commande.' },
      { kind: 'fixed', text: 'Giveaway StreamElements dont le lien vient d\'une autre page StreamElements (partenaire, autre nom) : il est suivi avec la page du lien, et se clôt bien tout seul.' },
      { kind: 'fixed', text: 'Synchronisation : les corrections faites sur un giveaway détecté il y a plus de 60 jours (gagnants…) partent aussi vers les autres PC.' },
      { kind: 'fixed', text: 'Ancien giveaway : créé directement terminé, jamais à moitié enregistré. Import par lot : une erreur sur un giveaway apparaît dans le rapport sans arrêter l\'import des autres.' },
      { kind: 'fixed', text: 'Journal du chat et filtre « Gagnant » : les majuscules accentuées sont trouvées (« été » trouve « ÉTÉ »).' },
      { kind: 'fixed', text: 'Web app : une réponse lente du serveur n\'écrase plus une plus récente (un giveaway fermé ne réapparaît plus).' }
    ]
  },
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
