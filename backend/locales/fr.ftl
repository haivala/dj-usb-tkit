# Catalogue de textes en français. Les clés et les commentaires de référence sont dans en.ftl ;
# une clé absente ici retombe sur l'anglais. Voir docs/TRANSLATING.md.

## status

status-warning-count = { $count ->
    [one] ({ $count } avertissement)
   *[other] ({ $count } avertissements)
}

## usb

usb-root-dialog-title = Choisir le dossier racine de la clé USB
usb-root-selected = Racine USB choisie : { $path }. Diagnostics en cours...
usb-root-cleared = Racine USB effacée
usb-root-selected-with-warnings = Racine USB choisie : { $path } ({ $warnings }). Diagnostics en cours...
usb-root-not-initialized = Clé USB choisie mais non initialisée. Cliquez sur « Initialiser la structure USB » pour continuer.
usb-root-init-hint-missing-structure = Le dossier USB est accessible en écriture mais n'a pas la structure External library
usb-root-init-hint-missing-structure-with-warnings = Le dossier USB est accessible en écriture mais n'a pas la structure External library ({ $warnings })
usb-root-invalid = Racine USB invalide : { $reason }
usb-root-invalid-unknown = racine USB invalide
usb-root-init-hint-not-ready = Le dossier USB n'est pas prêt pour l'initialisation
usb-root-init-hint-not-ready-with-warnings = Le dossier USB n'est pas prêt pour l'initialisation ({ $warnings })
usb-initialized = Clé USB initialisée
usb-drive-renamed = Clé renommée en { $name }.
usb-drive-name-unchanged = Nom de la clé inchangé.


usb-progress-resolving-root = USB : recherche de la racine
usb-progress-parsing-pdb = USB : lecture de la PDB
usb-progress-reading-edb = USB : lecture de l'eDB
usb-progress-resolving-edb-candidates = USB : recherche des candidats eDB
usb-progress-resolving-playlists = USB : recherche des playlists
usb-progress-finalizing-playlist-import = USB : finalisation de l'importation des playlists
usb-progress-resolving-usb-root = USB : recherche de la racine USB
usb-progress-backing-up-usb-databases = USB : sauvegarde des bases de données USB
usb-progress-order-edit-pdb = USB : écriture du nouvel ordre dans la PDB
usb-progress-order-edit-edb = USB : écriture du nouvel ordre dans l'eDB
usb-progress-order-edit-verify = USB : vérification du nouvel ordre dans les deux bases
usb-progress-order-edit-write-back = USB : enregistrement des bases sur la clé
usb-progress-order-edit-engine = USB : mise à jour de la bibliothèque Engine DJ
usb-progress-resolving-playlist-identifiers = USB : recherche des identifiants de playlists
usb-progress-analyzing-shared-tracks = USB : analyse des playlists et des morceaux partagés
usb-progress-deleting-exclusive-track-files = USB : suppression des fichiers propres à la playlist
usb-progress-cleaning-edb = USB : nettoyage de l'eDB
usb-progress-cleaning-empty-directories = USB : nettoyage des dossiers vides
usb-progress-finalizing = USB : finalisation
usb-progress-playlist-removal-completed = USB : suppression de la playlist terminée
usb-progress-reading-supplemental-databases = USB : lecture des bases de données complémentaires
usb-progress-building-track-index = USB : construction de l'index des morceaux
usb-progress-resolving-history-entries = USB : recherche des entrées d'historique
usb-progress-finalizing-history-import = USB : finalisation de l'importation de l'historique

usb-error-name-empty = Saisissez un nom pour cette clé.
usb-error-name-too-long = Le nom de la clé doit faire au plus { $max } caractères
usb-error-name-taken = Le nom de clé « { $name } » est déjà utilisé par une autre clé (« { $other } »)
usb-error-read-only-init = La clé USB est en lecture seule, elle ne peut donc pas être initialisée.
usb-error-no-player-database = Cette clé USB n'a pas de base de données pour les lecteurs. Exportez-y une playlist, ou préparez-la d'abord.
usb-error-no-pdb = Cette clé USB n'a que l'eDB, pas la PDB nécessaire ici. « Aperçu des corrections » peut créer la PDB à partir de l'eDB.
usb-error-track-not-found = morceau { $id } introuvable sur la clé USB

usb-forget-title = Supprimer { $name } de cet ordinateur ?
usb-forget-intro = Supprime de cet ordinateur :
usb-forget-nothing-kept = Cet ordinateur ne garde rien de cette clé, sauf sa place dans la liste.
usb-forget-tracks = { $count ->
    [one] { $count } morceau lu depuis cette clé
   *[other] { $count } morceaux lus depuis cette clé
}
usb-forget-cache = { $size } de copies de ses bases de données et { $backups ->
    [0] aucune sauvegarde
    [one] { $backups } sauvegarde
   *[other] { $backups } sauvegardes
}
usb-forget-usb-backup-stays = La sauvegarde la plus récente sur la clé USB elle-même reste.
usb-recent-no-cache-tooltip = { $path } : cet ordinateur ne garde aucune copie de ses bases de données ni de ses sauvegardes
usb-recent-no-folder-tooltip = Son dossier n'est pas connu : seul ce que cet ordinateur garde de cette clé est là
usb-error-device-not-listed = { $id } n'est plus dans la liste des clés USB récentes

usb-warning-path-empty = Le chemin USB est vide
usb-warning-path-not-found = Le chemin n'existe pas : { $path }
usb-warning-missing-vendor-root = Dossier racine du fabricant manquant
usb-warning-missing-contents = Dossier Contents manquant
usb-warning-read-only = La clé USB semble en lecture seule : les importations peuvent fonctionner, mais l'export échouera
usb-warning-pdb-not-found-edb-only = PDB introuvable sous { $root } ; poursuite en mode eDB seule
usb-warning-pdb-not-found-db-fallback = PDB introuvable sous { $root } ; utilisation de la base de secours uniquement
usb-warning-edb-unreadable = eDB illisible : ni SQLite simple, ni ouverte avec la clé d'export USB
usb-name-check-failed = Impossible de vérifier le nom de cette clé : { $error }
usb-audio-formats-read-failed = Impossible de lire les formats audio de la clé USB : { $error }
usb-initialize-failed = Échec de l'initialisation : { $error }

## usb-playlists

usb-playlists-summary = { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $tracks ->
    [one] { $tracks } morceau
   *[other] { $tracks } morceaux
}
usb-playlists-loaded = Playlists USB chargées : { $count }
usb-playlists-done = Terminé — { $summary }
usb-playlists-removed = Playlist USB retirée : { $name } [db { $edb }, pdb { $pdb }]

usb-playlists-selected = Playlist USB sélectionnée : { $name } ({ $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
})

usb-playlists-remove-confirm = Retirer la playlist USB « { $name } » de la clé ?

usb-playlists-warning-empty-source = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} sans aucune entrée de morceau statique dans les données d'export : { $names }
usb-playlists-warning-shared-tracks-kept = { $count ->
    [one] { $count } morceau partagé conservé (utilisé par d'autres playlists)
   *[other] { $count } morceaux partagés conservés (utilisés par d'autres playlists)
}
usb-playlists-warning-files-missing = { $count ->
    [one] { $count } fichier déjà absent de la clé USB
   *[other] { $count } fichiers déjà absents de la clé USB
}
usb-playlists-warning-file-deleted = supprimé : { $path }
usb-playlists-warning-edb-removal-skipped = Suppression de la playlist dans l'eDB ignorée : l'eDB n'a pas pu être ouverte
usb-playlists-warning-pdb-removal-skipped = Suppression de la playlist dans la PDB ignorée : PDB manquante
usb-playlists-remove-failed = Échec du retrait de la playlist USB : { $error }

usb-order-edit-editing = Modification de l'ordre : rien n'a encore changé
usb-order-edit-editing-changed = Modification de l'ordre : { $count ->
    [one] { $count } playlist modifiée
   *[other] { $count } playlists modifiées
}
usb-order-edit-editing-list-changed = Modification de l'ordre : ordre des playlists modifié
usb-order-edit-editing-list-and-changed = Modification de l'ordre : ordre des playlists et { $count ->
    [one] { $count } playlist modifiés
   *[other] { $count } playlists modifiés
}
usb-order-edit-started = Modification de l'ordre : faites glisser les playlists et les morceaux, ou triez une playlist par colonne. Rien n'est écrit avant « Enregistrer l'ordre ».
usb-order-edit-saved = Ordre enregistré sur la clé USB
usb-order-edit-unchanged = Ordre inchangé : rien à enregistrer
usb-order-edit-cancelled = Modifications de l'ordre abandonnées
usb-order-edit-not-active = La modification de l'ordre n'est pas active pour cette clé USB
usb-order-edit-move-into-playlist = Les éléments vont dans des dossiers, pas dans des playlists
usb-order-edit-move-into-itself = Un dossier ne peut pas aller dans lui-même ni dans un dossier qu’il contient
usb-order-edit-move-name-taken = Il y a déjà une playlist ou un dossier nommé « { $name } » à cet endroit
usb-order-edit-saving = Le nouvel ordre est en cours d'enregistrement sur la clé USB ; attendez la fin
usb-write-busy = Une autre écriture USB est en cours ; réessayez quand elle sera terminée
usb-order-edit-read-only = Cette clé USB n'est pas accessible en écriture, son ordre ne peut donc pas être modifié
usb-order-edit-history-refused = L'ordre d'une liste d'historique est l'ordre de lecture ; il ne peut pas être modifié
usb-order-edit-track-not-found = Le morceau { $id } n'est pas dans cette playlist USB
usb-order-edit-edb-open-failed = L'eDB de la clé USB n'a pas pu être ouverte, rien n'a donc été enregistré
usb-edb-checkpoint-failed = L'eDB n'a pas pu finir d'écrire ses modifications, rien n'a donc été enregistré
usb-order-edit-save-failed = Échec de l'enregistrement de l'ordre : { $error }
usb-order-edit-folder-missing = « { $item } » ne peut pas aller dans « { $folder } » : la { $database } de la clé USB n'a pas ce dossier, rien n'a donc été enregistré sur la clé USB. Lancez « Aperçu des corrections » dans « État et diagnostics », appliquez « Mettre les données d'export en parité stricte », puis enregistrez à nouveau l'ordre.
usb-order-edit-verify-failed = Le nouvel ordre est différent dans la PDB et dans l'eDB pour { $playlists }, rien n'a donc été enregistré sur la clé USB. Lancez « Aperçu des corrections » dans « État et diagnostics », appliquez « Mettre les données d'export en parité stricte », puis enregistrez à nouveau l'ordre.
usb-order-edit-warning-pdb-duplicates = Ordre des morceaux dans la PDB inchangé pour { $playlist } : la PDB y liste un morceau deux fois (les diagnostics USB peuvent le réparer)

## usb-history

usb-history-summary = { $sessions ->
    [one] { $sessions } session
   *[other] { $sessions } sessions
}, { $tracks ->
    [one] { $tracks } morceau
   *[other] { $tracks } morceaux
}

usb-history-loaded = Historiques USB chargés : { $sessions }
usb-history-heading = { $name } ({ $databases })
usb-history-source-both = Les deux bases
usb-history-warning-pdb-not-found = PDB introuvable sous { $root } ; l'importation de l'historique nécessite la PDB

## usb-player-menu

usb-player-menu-loaded = Menu du lecteur chargé
usb-player-menu-updated = Menu du lecteur mis à jour
usb-player-menu-unchanged = Menu du lecteur inchangé
usb-player-menu-unnamed = Menu { $kind }

usb-player-menu-error-no-edb = impossible d'ouvrir l'eDB de la clé USB pour le menu du lecteur
usb-player-menu-error-tables = tables menuItem/category absentes de l'eDB
usb-player-menu-error-no-menu = cette clé USB n'a pas de menu du lecteur dans son eDB ni dans sa PDB
usb-player-menu-error-cant-change = le menu du lecteur de cette clé USB ne peut pas être modifié : elle n'a pas d'eDB et aucune ligne de menu dans sa PDB
usb-player-menu-error-cant-remove = L'élément « { $name } » du menu du lecteur (type { $kind }) ne peut pas être retiré
usb-player-menu-warning-databases-differ = La PDB et l'eDB montrent des menus du lecteur différents. Affichage de celui de l'eDB : la prochaine modification du menu l'écrit dans les deux.
usb-player-menu-warning-edb-write-failed = Échec de l'écriture de l'eDB ({ $error }) ; restauration de l'eDB depuis l'instantané

## usb-background-color

usb-background-color-default = Par défaut
usb-background-color-pink = Rose
usb-background-color-red = Rouge
usb-background-color-orange = Orange
usb-background-color-yellow = Jaune
usb-background-color-green = Vert
usb-background-color-aqua = Turquoise
usb-background-color-blue = Bleu
usb-background-color-purple = Violet
usb-background-color-numbered = couleur { $color }
usb-background-color-two-colors = rekordbox a défini deux couleurs : { $pdb } pour les anciens lecteurs (PDB), { $edb } pour les nouveaux lecteurs et rekordbox (eDB). Choisir une couleur règle les deux.
usb-background-color-no-place = Cette clé USB n'a pas d'emplacement pour une couleur de fond.
usb-background-color-read-only = Cette clé USB est en lecture seule, sa couleur de fond ne peut donc pas être modifiée.
usb-background-color-set = Couleur de fond réglée sur { $color }
usb-background-color-unchanged = Couleur de fond inchangée

usb-background-color-error-no-edb = impossible d'ouvrir l'eDB de la clé USB
usb-background-color-read-failed = Impossible de lire la couleur de fond : { $error }

## usb-audio-formats

usb-audio-formats-hint = Les formats que l'export met sur cette clé USB · glisser pour définir l'ordre de préférence
usb-audio-formats-read-only = La clé USB est en lecture seule
usb-audio-formats-last-ticked = Au moins un format reste coché

usb-audio-formats-error-read-only = La clé USB est en lecture seule, ses formats audio ne peuvent donc pas être modifiés
usb-audio-formats-save-failed = Échec de l'enregistrement des formats audio USB : { $error }

## backups

backups-summary = { $count ->
    [one] { $count } sauvegarde
   *[other] { $count } sauvegardes
}
backups-empty = Aucune sauvegarde pour l'instant.
backups-files-both = eDB et PDB
backups-files-edb = eDB
backups-files-pdb = PDB
backups-files-none = Sauvegarde
backups-on-usb = Sur la clé USB
backups-on-computer = Sur cet ordinateur
backups-playlists = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
}
backups-restore-confirm = Restaurer { $files } depuis la sauvegarde faite le { $time } ? Les fichiers actuels seront d'abord sauvegardés eux aussi.
backups-delete-confirm = Supprimer définitivement cette sauvegarde { $files } ?
backups-restored = { $files } restaurée(s) depuis la sauvegarde
backups-deleted = Sauvegarde { $files } supprimée
backups-reason-export = Avant l'export
backups-reason-export-mirror = Avant l'export (miroir)
backups-reason-export-additive = Avant l'export (additif)
backups-reason-playlist-reorder = Avant le réordonnancement des playlists
backups-reason-order-edit = Avant la modification de l'ordre
backups-reason-playlist-removal = Avant le retrait d'une playlist
backups-reason-menu-update = Avant la mise à jour du menu
backups-reason-background-color = Avant le changement de couleur de fond
backups-reason-repair = Avant la réparation
backups-reason-restore = Avant la restauration

backups-error-not-found = sauvegarde introuvable : { $backup }

backups-note-written = Sauvegarde : { $file }
backups-warning-folder-failed = Sauvegarde ignorée : impossible de créer le dossier des sauvegardes : { $error }
backups-warning-copy-failed = Échec de la sauvegarde de { $file } : { $error }
backups-restore-failed = Échec de la restauration : { $error }
backups-delete-failed = Échec de la suppression : { $error }

## diagnostics

diagnostics-status-pass = OK
diagnostics-status-warn = ATTENTION
diagnostics-status-fail = ÉCHEC
diagnostics-health-good = État de la clé USB : bon
diagnostics-health-warnings = État de la clé USB : avertissements
diagnostics-health-issues = État de la clé USB : problèmes détectés
diagnostics-complete = Diagnostics terminés ({ $ms } ms)
diagnostics-duration = Terminé en { $ms } ms
parity-complete = Rapport de parité terminé ({ $ms } ms)
parity-status-label = Parité { $status }

diagnostics-group-databases = Bases de données
diagnostics-group-library = Bibliothèque
diagnostics-group-low-level = Bas niveau
diagnostics-group-comparison = Comparaison
diagnostics-group-drive = Support
diagnostics-section-pdb = PDB
diagnostics-section-edb = eDB
diagnostics-section-engine = Engine DJ
diagnostics-section-analysis = Analyse
diagnostics-section-player-counters = Compteurs du lecteur
diagnostics-section-page-format = Format des pages
diagnostics-section-drive-format = Partitions et système de fichiers

diagnostics-area-older-players = Anciens lecteurs Pioneer
diagnostics-area-newer-players = Nouveaux lecteurs Pioneer
diagnostics-area-playlists = Playlists
diagnostics-area-track-details = Détails des morceaux
diagnostics-area-music-files = Fichiers audio
diagnostics-area-analysis = Formes d'onde et beatgrids
diagnostics-area-engine = Lecteurs Denon / Numark
diagnostics-area-drive = Clé USB
diagnostics-preview-fixes-hint = « Aperçu des corrections » montre ce qui peut être réparé.

diagnostics-overview-pdb-missing = Cette clé USB n'a pas de base de données pour les anciens lecteurs, ils risquent donc de ne pas afficher sa musique.
diagnostics-overview-pdb-unreadable = La base de données des lecteurs est absente ou illisible, ces lecteurs n'afficheront donc aucune musique.
diagnostics-overview-pdb-ready = Prête : { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $tracks ->
    [one] { $tracks } morceau
   *[other] { $tracks } morceaux
}.
diagnostics-overview-pdb-track-count = Ces lecteurs afficheront 0 ou 1 morceau : une partie de la base de données des lecteurs ne correspond pas à la musique de cette clé USB.
diagnostics-overview-pdb-irregular = Lisible, mais certaines parties de la base de données des lecteurs sont irrégulières et les logiciels DJ de bureau peuvent s'en plaindre.
diagnostics-overview-pdb-errors = La base de données des lecteurs contient des erreurs qui peuvent figer ces lecteurs ou leur faire afficher de mauvaises données.
diagnostics-overview-edb-missing = Cette clé USB n'a pas de base de données pour les nouveaux lecteurs, ils risquent donc de ne pas afficher sa musique.
diagnostics-overview-edb-not-yet = Cette clé USB n'a pas encore de base de données pour les nouveaux lecteurs.
diagnostics-overview-edb-unreadable = La base de données que lisent les nouveaux lecteurs ne peut pas être ouverte, ils risquent donc de ne pas afficher la musique de cette clé USB.
diagnostics-overview-edb-pdb-only = { $count ->
    [one] { $count } morceau manque dans la base de données que lisent les nouveaux lecteurs, il risque donc de ne pas y apparaître.
   *[other] { $count } morceaux manquent dans la base de données que lisent les nouveaux lecteurs, ils risquent donc de ne pas y apparaître.
}
diagnostics-overview-edb-edb-only = { $count ->
    [one] { $count } morceau n'est que dans la base de données des nouveaux lecteurs, les anciens lecteurs Pioneer ne l'afficheront donc pas.
   *[other] { $count } morceaux ne sont que dans la base de données des nouveaux lecteurs, les anciens lecteurs Pioneer ne les afficheront donc pas.
}
diagnostics-overview-edb-bpm-key = Certains morceaux peuvent afficher un BPM ou une tonalité différents sur les nouveaux lecteurs et sur les anciens.
diagnostics-overview-edb-analysis = Certains morceaux peuvent n'avoir ni forme d'onde ni beatgrid sur les nouveaux lecteurs.
diagnostics-overview-edb-irregular = Certaines parties de la base de données des nouveaux lecteurs sont irrégulières.
diagnostics-overview-edb-ready = Prête : { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}.
diagnostics-overview-playlists-ready = Tous les morceaux de vos { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} sont trouvés.
diagnostics-overview-playlists-missing = { $count ->
    [one] { $count } morceau de vos playlists est introuvable.
   *[other] { $count } morceaux de vos playlists sont introuvables.
}
diagnostics-overview-playlists-mismatch = Certaines données de playlists ne concordent pas entre les bases de données.
diagnostics-overview-analysis-ready = Tous les morceaux ont leurs données de forme d'onde et de beatgrid.
diagnostics-overview-analysis-problems = Les données de forme d'onde ou de beatgrid de certains morceaux sont absentes ou obsolètes.
diagnostics-overview-engine-next-change = Sera ajoutée la prochaine fois que l'application modifiera la clé USB.
diagnostics-overview-engine-not-set-up = Non configurée. Nécessaire uniquement pour les lecteurs Denon Prime / Numark Mixstream : activez Paramètres → Export → « Écrire la bibliothèque Engine DJ ».
diagnostics-overview-engine-updates-off = Les mises à jour sont désactivées, les modifications faites dans l'application n'atteignent donc pas ces lecteurs.
diagnostics-overview-engine-out-of-date = Obsolète, le lecteur va donc analyser à nouveau chaque morceau.
diagnostics-overview-engine-ready = Prête.
diagnostics-overview-drive-ready = Partitionnée et formatée comme les lecteurs l'attendent.
diagnostics-overview-drive-layout = Partitionnée ou formatée d'une façon que tous les lecteurs ne lisent pas. Si un lecteur propose de formater cette clé USB ou ne l'affiche pas, reformatez-la comme l'indique « Partitions et système de fichiers » ci-dessous.

parity-overview-playlist-ids = { $count ->
    [one] { $count } playlist ne concorde pas entre les deux bases de données.
   *[other] { $count } playlists ne concordent pas entre les deux bases de données.
}
parity-overview-only-in-pdb = { $count ->
    [one] { $count } morceau de playlist n'est que dans la base de données que lisent les anciens lecteurs Pioneer.
   *[other] { $count } morceaux de playlists ne sont que dans la base de données que lisent les anciens lecteurs Pioneer.
}
parity-overview-only-in-edb = { $count ->
    [one] { $count } morceau de playlist n'est que dans la base de données que lisent les nouveaux lecteurs Pioneer.
   *[other] { $count } morceaux de playlists ne sont que dans la base de données que lisent les nouveaux lecteurs Pioneer.
}
parity-overview-order = { $count ->
    [one] { $count } playlist est dans un ordre différent selon le lecteur.
   *[other] { $count } playlists sont dans un ordre différent selon le lecteur.
}
parity-overview-duplicates = { $count ->
    [one] { $count } morceau est listé deux fois dans une playlist sur les anciens lecteurs Pioneer.
   *[other] { $count } morceaux sont listés deux fois dans une playlist sur les anciens lecteurs Pioneer.
}
parity-overview-pdb-details = { $count ->
    [one] { $count } morceau n'a pas tous ses détails sur les anciens lecteurs Pioneer.
   *[other] { $count } morceaux n'ont pas tous leurs détails sur les anciens lecteurs Pioneer.
}
parity-overview-edb-details = { $count ->
    [one] { $count } morceau n'a pas tous ses détails dans la base de données que lisent les nouveaux lecteurs Pioneer.
   *[other] { $count } morceaux n'ont pas tous leurs détails dans la base de données que lisent les nouveaux lecteurs Pioneer.
}
parity-overview-dictionary = { $count ->
    [one] { $count } morceau renvoie à un artiste, un album ou une tonalité absent des anciens lecteurs Pioneer.
   *[other] { $count } morceaux renvoient à un artiste, un album ou une tonalité absent des anciens lecteurs Pioneer.
}
parity-overview-artwork = { $count ->
    [one] { $count } morceau a une pochette sur un type de lecteur Pioneer mais pas sur l'autre.
   *[other] { $count } morceaux ont une pochette sur un type de lecteur Pioneer mais pas sur l'autre.
}
parity-overview-rating-colour = { $count ->
    [one] { $count } morceau a une note ou une couleur différente sur les anciens lecteurs Pioneer.
   *[other] { $count } morceaux ont une note ou une couleur différente sur les anciens lecteurs Pioneer.
}
parity-overview-files-missing = { $count ->
    [one] { $count } fichier audio manque sur la clé USB, ce morceau ne sera donc pas lu.
   *[other] { $count } fichiers audio manquent sur la clé USB, ces morceaux ne seront donc pas lus.
}
parity-overview-files-extra = { $count ->
    [one] { $count } fichier audio de la clé USB n'est dans aucune playlist.
   *[other] { $count } fichiers audio de la clé USB ne sont dans aucune playlist.
}
parity-overview-paths = { $count ->
    [one] { $count } morceau pointe vers un fichier audio ou de forme d'onde différent selon le lecteur.
   *[other] { $count } morceaux pointent vers un fichier audio ou de forme d'onde différent selon le lecteur.
}
parity-overview-pdb-missing = Cette clé USB a une eDB mais pas de PDB, il n'y a donc rien à comparer.
parity-overview-edb-missing = Cette clé USB a une PDB mais pas d'eDB, il n'y a donc rien à comparer.
parity-overview-playlists-ready = Les deux bases de données ont les mêmes { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
}, avec les mêmes morceaux dans le même ordre.
parity-overview-tracks-ready = Chaque morceau de playlist a des détails complets et la même pochette dans les deux bases de données.
parity-overview-files-ready = Le fichier audio de chaque morceau de playlist est sur la clé USB, et les deux bases de données pointent vers les mêmes fichiers.

parity-issue-only-in-pdb = +PDB { $count }
parity-issue-only-in-edb = +eDB { $count }
parity-issue-duplicates = doublons PDB { $count }
parity-issue-pdb-gaps = lacunes PDB { $count }
parity-issue-edb-gaps = lacunes eDB { $count }
parity-issue-path-mismatch = chemins différents { $count }
parity-issue-dictionary = problèmes de dict. { $count }
parity-issue-artwork = pochettes différentes { $count }
parity-issue-rating-colour = note/couleur différente { $count }
parity-issue-field-notes = notes de champs { $count }
parity-issue-same-name-copies = copies de même nom { $count }
parity-issue-in-one-database = dans une seule base
parity-issue-sort-mismatch = tri différent
parity-issue-order-mismatch = ordre différent

diagnostics-check-pdb-exists = PDB présente
diagnostics-check-pdb-exists-found = Trouvée
diagnostics-check-pdb-parseable = PDB lisible
diagnostics-check-pdb-parseable-error = Erreur de lecture : { $error }
diagnostics-check-pdb-parseable-counts = { $tracks ->
    [one] { $tracks } morceau
   *[other] { $tracks } morceaux
}, { $artists ->
    [one] { $artists } artiste
   *[other] { $artists } artistes
}, { $albums ->
    [one] { $albums } album
   *[other] { $albums } albums
}, { $genres ->
    [one] { $genres } genre
   *[other] { $genres } genres
}, { $labels ->
    [one] { $labels } label
   *[other] { $labels } labels
}, { $keys ->
    [one] { $keys } tonalité
   *[other] { $keys } tonalités
}, { $artworks ->
    [one] { $artworks } pochette
   *[other] { $artworks } pochettes
}
diagnostics-check-pdb-header = Compatibilité de l'en-tête PDB
diagnostics-check-pdb-header-value = valeur { $value } aux octets 0x10..0x14
diagnostics-check-pdb-header-unknown = valeur { $value } aux octets 0x10..0x14 ; une valeur compatible connue était attendue
diagnostics-check-pdb-page-headers = En-têtes de pages PDB
diagnostics-check-pdb-page-headers-valid = u5, page_flags et tranrf sont valides sur toutes les pages de données
diagnostics-check-pdb-page-headers-damaged = { $count ->
    [one] { $count } page
   *[other] { $count } pages
} avec des page_flags invalides ou un nombre de lignes actives erroné
diagnostics-check-pdb-page-headers-footer = { $count ->
    [one] { $count } page
   *[other] { $count } pages
} dont u5/num_rl ne correspondent pas à tranrf
diagnostics-check-pdb-structure = Intégrité structurelle de la PDB
diagnostics-check-pdb-structure-ok = b-tree sentinelle, formes des pages, ID de lignes et pointeurs d'écriture OK
diagnostics-check-pdb-structure-ok-bookkeeping = b-tree sentinelle, formes des pages, ID de lignes et pointeurs d'écriture OK ; suivi des transactions de rekordbox : { $parts }
diagnostics-check-pdb-structure-fails = { $parts } — lancez repair_usb_diagnostics pour corriger
diagnostics-check-pdb-structure-stale-btree = { $count ->
    [one] { $count } sentinelle
   *[other] { $count } sentinelles
} b-tree obsolète
diagnostics-check-pdb-structure-ec-conflict = { $count ->
    [one] { $count } table
   *[other] { $count } tables
} en conflit de pointeur d'écriture
diagnostics-check-pdb-structure-truncated = { $count ->
    [one] { $count } table
   *[other] { $count } tables
} dont la dernière page déclarée dépasse la fin physique du fichier
diagnostics-check-pdb-structure-torn-pages = { $count ->
    [one] { $count } page empty_candidate
   *[other] { $count } pages empty_candidate
} déchirée(s) par un export interrompu
diagnostics-check-pdb-structure-torn-tail = Fin de la PDB déchirée par un export interrompu
diagnostics-check-pdb-playlists = Playlists PDB
diagnostics-check-pdb-playlists-counts = { $nodes ->
    [one] { $nodes } nœud d'arborescence
   *[other] { $nodes } nœuds d'arborescence
}, { $entries ->
    [one] { $entries } entrée
   *[other] { $entries } entrées
}
diagnostics-check-pdb-history = Historique PDB
diagnostics-check-pdb-history-counts = { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $entries ->
    [one] { $entries } entrée
   *[other] { $entries } entrées
}, { $rows ->
    [one] { $rows } ligne
   *[other] { $rows } lignes
}
diagnostics-check-history-empty = vide
diagnostics-check-num-rl-pages = pages num_rl=8191
diagnostics-check-num-rl-pages-detail = { $count } sur { $total ->
    [one] { $total } page
   *[other] { $total } pages
} (repli sur nrs)
diagnostics-check-nrs-wrapping = Débordement de nrs
diagnostics-check-nrs-wrapping-detail = { $count ->
    [one] { $count } page
   *[other] { $count } pages
} dont le nombre de lignes dépasse l'en-tête nrs
diagnostics-check-orphaned-entries = Entrées orphelines
diagnostics-check-orphaned-entries-detail = { $entries ->
    [one] { $entries } entrée
   *[other] { $entries } entrées
} renvoyant à { $tracks ->
    [one] { $tracks } ID de morceau
   *[other] { $tracks } ID de morceaux
} absent(s) de la PDB
diagnostics-check-empty-duplicate-playlists = Playlists vides en double
diagnostics-check-empty-duplicate-playlists-detail = { $count ->
    [0] aucune playlist vide ne porte le nom d'une playlist avec des morceaux
    [one] { $count } playlist vide porte le nom d'une playlist avec des morceaux ; « Aperçu des corrections » la supprime
   *[other] { $count } playlists vides portent le nom d'une playlist avec des morceaux ; « Aperçu des corrections » les supprime
}
diagnostics-check-parse-warnings = Avertissements de lecture
diagnostics-check-parse-warnings-detail = { $count ->
    [one] { $count } avertissement
   *[other] { $count } avertissements
} pendant la lecture

diagnostics-check-edb = eDB
diagnostics-check-edb-missing = Fichier introuvable
diagnostics-check-edb-opened = Ouverte avec succès
diagnostics-check-edb-unable = Impossible d'ouvrir
diagnostics-check-pdb-paths = Chemins indexés de la PDB
diagnostics-check-edb-paths = Chemins indexés de l'eDB
diagnostics-check-paths-count = { $count ->
    [one] { $count } chemin de morceau canonique
   *[other] { $count } chemins de morceaux canoniques
}
diagnostics-check-path-agreement = Concordance des chemins PDB/eDB
diagnostics-check-path-agreement-agree = Les ensembles de chemins canoniques PDB/eDB concordent
diagnostics-check-path-agreement-pdb-only = { $count ->
    [one] { $count } chemin seulement dans la PDB
   *[other] { $count } chemins seulement dans la PDB
}
diagnostics-check-path-agreement-edb-only = { $count ->
    [one] { $count } chemin seulement dans l'eDB
   *[other] { $count } chemins seulement dans l'eDB
}
diagnostics-check-path-agreement-history-only = { $count ->
    [one] { $count } morceau seulement dans l'historique
   *[other] { $count } morceaux seulement dans l'historique
}
diagnostics-check-beat-grid = Format des beatgrids
diagnostics-check-beat-grid-not-exported = aucun morceau de cette clé USB n'a été exporté par cette application
diagnostics-check-beat-grid-current = toutes les beatgrids que cette application a écrites sur cette clé USB sont au format actuel
diagnostics-check-beat-grid-outdated = Morceaux dont la beatgrid est dans un format obsolète que certains lecteurs (p. ex. le XDJ-AZ) refusent, si bien que le Beat Sync n'y est pas disponible : { $count } ; réexportez ou lancez la réparation (update_app_analysis_files) pour corriger. Voir le Journal des événements pour la liste.
diagnostics-check-long-waveforms = Formes d'onde des morceaux longs
diagnostics-check-long-waveforms-ok = aucun morceau de plus d'environ 9 minutes n'a de forme d'onde tracée par l'analyse jusqu'à 0.6.6
diagnostics-check-long-waveforms-stretched = Morceaux de plus d'environ 9 minutes dont l'analyse jusqu'à 0.6.6 a tracé la forme d'onde étirée, si bien que le son apparaît plus tard qu'il ne joue et que la fin manque : { $count } ; lancez la réparation (update_app_analysis_files) pour corriger. Voir le Journal des événements pour la liste.
diagnostics-check-pdb-analysis = Références d'analyse PDB
diagnostics-check-pdb-analysis-detail = { $with }/{ $total } lignes de morceaux avec chemin d'analyse ; { $refs ->
    [one] { $refs } référence de bundle unique
   *[other] { $refs } références de bundles uniques
}
diagnostics-check-no-pdb-data = Aucune donnée PDB disponible
diagnostics-check-edb-analysis = Références d'analyse eDB
diagnostics-check-edb-analysis-detail = { $with }/{ $total } lignes de contenu liées à une playlist avec chemin d'analyse ; { $refs ->
    [one] { $refs } référence de bundle unique
   *[other] { $refs } références de bundles uniques
} pour { $entries ->
    [one] { $entries } entrée de playlist
   *[other] { $entries } entrées de playlists
}
diagnostics-check-edb-analysis-no-tracks = Aucun morceau dans l'eDB
diagnostics-check-edb-analysis-no-data = Aucune donnée de playlist eDB disponible
diagnostics-check-playlist-data = Données des playlists
diagnostics-warning-playlist-edb-only = { $playlist } : présente seulement dans l'eDB ({ $count ->
    [one] { $count } entrée
   *[other] { $count } entrées
}), absente de la PDB
diagnostics-check-overall-resolution = Résolution globale
diagnostics-check-overall-resolution-detail = { $resolved }/{ $total } entrées résolues ({ NUMBER($percent) } %) sur { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}
diagnostics-check-db-only-playlists = Playlists seulement dans la base
diagnostics-check-db-only-playlists-detail = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} dans l'eDB mais pas dans la PDB. Voir le Journal des événements pour la liste.
diagnostics-check-unresolved-playlists = Playlists non résolues
diagnostics-check-unresolved-playlists-detail = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} avec des entrées dont le morceau manque. Voir le Journal des événements pour la liste.
diagnostics-check-key-overlap = Recouvrement des clés PDB et eDB (pour information)
diagnostics-check-key-overlap-detail = { $matched } clés de morceaux concordantes ; PDB { NUMBER($pdbshare) } % ({ $matched }/{ $pdb }), DB { NUMBER($edbshare) } % ({ $matched }/{ $edb })
diagnostics-check-operational = Interprétation opérationnelle
diagnostics-check-operational-detail = OK/attention signifie ici utilisable en pratique ; la parité stricte peut quand même échouer sur la même clé USB
diagnostics-warning-playlist-unresolved = { $playlist } : { $resolved }/{ $total } entrées résolues ({ NUMBER($percent) } %)

diagnostics-check-history-page = Page d'historique PDB
diagnostics-check-history-page-mismatch = t19 ne correspond pas au nombre de morceaux ni à la ligne de propriétés de l'eDB — les anciens lecteurs peuvent afficher 0 ou 1 morceau. La réparation corrige cela.
diagnostics-check-predicted-counter = Compteur prévu du lecteur
diagnostics-check-predicted-counter-detail = playlists { $playlists }, morceaux { $songs } (confiance : { $confidence })
diagnostics-check-shape-mode = Mode de forme
diagnostics-check-shape-mode-detail = { $mode } (proche de baseline-init : { $baseline })
diagnostics-check-yes = oui
diagnostics-check-no = non
diagnostics-check-cross-check = Contre-vérification
diagnostics-check-cross-check-detail = morceaux t00 { $tracks }, entrées t08 { $entries }
diagnostics-check-history-pointers = Pointeurs d'historique
diagnostics-check-primitive-signal = Signal primitif

diagnostics-check-edb-history = Historique eDB
diagnostics-check-edb-history-counts = { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
}, { $entries ->
    [one] { $entries } entrée
   *[other] { $entries } entrées
}
diagnostics-check-string-alignment = Alignement des chaînes morceau/album
diagnostics-check-string-alignment-ok = Tous les emplacements de chaînes UTF-16 et ASCII longues sont alignés sur 4 octets
diagnostics-check-string-alignment-misaligned = { $count ->
    [one] { $count } ligne
   *[other] { $count } lignes
} avec un emplacement de chaîne UTF-16 ou ASCII longue mal aligné — fige le CDJ ou provoque des erreurs de communication pendant la navigation ; lancez repair_pdb_track_string_alignment / repair_pdb_album_string_alignment / repair_pdb_artist_name_layout pour corriger. Voir le Journal des événements pour la liste.
diagnostics-check-artist-layout = Disposition des noms d'artistes
diagnostics-check-artist-layout-ok = Tous les noms d'artistes sont là où rekordbox les place
diagnostics-check-artist-layout-moved = { $count ->
    [one] { $count } ligne d'artiste
   *[other] { $count } lignes d'artistes
} avec le nom à un autre décalage que celui de rekordbox ; un lecteur Engine DJ qui importe export.pdb affiche les morceaux de l'artiste sans artiste ; lancez repair_pdb_artist_name_layout pour corriger
diagnostics-check-bpm-key = Cohérence BPM/tonalité
diagnostics-check-bpm-key-ok = export.pdb et exportLibrary.db concordent sur le BPM et la tonalité de chaque morceau
diagnostics-check-bpm-key-mismatch = { $count ->
    [one] { $count } fichier audio
   *[other] { $count } fichiers audio
} dont les lignes dans export.pdb et exportLibrary.db divergent sur le BPM ou la tonalité -- les CDJ peuvent afficher un autre BPM/une autre tonalité que l'application ; lancez la réparation (fix_bpm_key_mismatch, qui prend le BPM dans la beatgrid du fichier d'analyse) pour corriger. Voir le Journal des événements pour la liste.
diagnostics-check-shared-analysis = Fichiers d'analyse partagés
diagnostics-check-shared-analysis-ok = chaque morceau a son propre fichier d'analyse
diagnostics-check-shared-analysis-shared = { $tracks ->
    [one] { $tracks } morceau
   *[other] { $tracks } morceaux
} partageant { $files ->
    [one] { $files } fichier d'analyse
   *[other] { $files } fichiers d'analyse
}, les lecteurs peuvent donc afficher la forme d'onde et la beatgrid d'un morceau pour un autre ; lancez la réparation (update_app_analysis_files) pour corriger. Voir le Journal des événements pour la liste.
diagnostics-check-engine-missing = Absente de cette clé USB
diagnostics-check-engine-missing-detail = Nécessaire uniquement pour les lecteurs Denon Prime / Numark Mixstream. Activez Paramètres → Export → « Écrire la bibliothèque Engine DJ » avant d'utiliser cette clé USB dans l'un d'eux ; sinon le lecteur importe lui-même export.pdb et analyse à nouveau chaque morceau
diagnostics-check-engine-upkeep = Tenue à jour
diagnostics-check-engine-upkeep-on = Reconstruite chaque fois que l'application modifie la clé USB
diagnostics-check-engine-upkeep-off = « Écrire la bibliothèque Engine DJ » est désactivé, les modifications que l'application apporte à la clé USB n'atteignent donc pas les lecteurs Denon Prime / Numark Mixstream. « Aperçu des corrections » peut l'activer
diagnostics-check-engine-sync = Correspond à export.pdb
diagnostics-check-engine-sync-ok = Écrite pour export.pdb n° { $pdb }
diagnostics-check-engine-sync-no-pdb = Écrite sans PDB : la clé USB n'en a pas à importer pour le lecteur
diagnostics-check-engine-sync-behind = Écrite pour export.pdb n° { $library }, la clé USB a le n° { $pdb }. Le lecteur peut importer à nouveau export.pdb et analyser lui-même les morceaux. « Aperçu des corrections » peut la reconstruire
diagnostics-check-drive-layout = Partitionnement
diagnostics-check-drive-layout-ok = Une partition MBR en FAT32, comme les lecteurs l'attendent
diagnostics-check-drive-layout-reformat = Pour que tous les lecteurs la lisent, sauvegardez la clé USB, reformatez-la avec une seule partition MBR en FAT32 (« Preparing a USB Drive » dans la documentation de l'application donne les étapes), puis recopiez tout dessus.
diagnostics-drive-older-fat = Le système de fichiers est { $version }, pas FAT32.
diagnostics-drive-exfat = Le système de fichiers est exFAT, que seuls les lecteurs récents lisent.
diagnostics-drive-other-fs = Le système de fichiers est { $name }, que la plupart des lecteurs ne savent pas lire.
diagnostics-drive-gpt = La table de partitions est GPT, que tous les lecteurs ne lisent pas.
diagnostics-drive-no-table = La clé n'a pas de table de partitions : le système de fichiers occupe toute la clé.
diagnostics-drive-other-table = La table de partitions est { $name }, pas MBR.
diagnostics-drive-not-first = C'est la partition { $number } de la clé ; les lecteurs ne regardent que la première.
diagnostics-drive-more-partitions = La clé a { $count } partitions ; les lecteurs ne regardent que la première.
diagnostics-drive-fs-smaller = Le système de fichiers fait { $filesystem } dans une partition de { $partition }.

diagnostics-check-parity-overall = État global de parité des lecteurs
diagnostics-check-parity-overall-detail = playlists vérifiées : { $playlists }, playlists en échec : { $failing }, id de playlists différents : { $ids }
diagnostics-check-parity-section = Section du rapport de parité (obligatoire)
diagnostics-check-parity-section-detail = Voir les lignes de synthèse de parité pour le décompte par catégorie.
diagnostics-check-parity-identity = Parité de l'identité des playlists
diagnostics-check-parity-identity-detail = { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} dont l'id diffère ou n'est pas résolu entre la PDB et l'eDB
diagnostics-check-parity-membership = Parité du contenu des playlists
diagnostics-check-parity-membership-detail = écarts de contenu : seulement-dans-PDB={ $pdb }, seulement-dans-eDB={ $edb }
diagnostics-check-parity-ordering = Parité de l'ordre des playlists
diagnostics-check-parity-ordering-detail = ordre des entrées différent={ $entries }, ordre de tri des playlists différent={ $playlists }
diagnostics-check-parity-files = Présence des fichiers audio indexés
diagnostics-check-parity-files-ok = les { $total } fichiers audio indexés sont présents, aucun fichier non indexé
diagnostics-check-parity-files-missing = { $missing } sur { $total ->
    [one] { $total } fichier audio indexé
   *[other] { $total } fichiers audio indexés
} absent(s) de la clé USB
diagnostics-check-parity-files-extra = { $extra ->
    [one] { $extra } fichier audio
   *[other] { $extra } fichiers audio
} sur la clé USB référencé(s) dans aucune playlist (total indexé={ $total })
diagnostics-check-parity-files-both = { $missing } sur { $total ->
    [one] { $total } fichier indexé
   *[other] { $total } fichiers indexés
} absent(s) ; { $extra ->
    [one] { $extra } fichier non indexé
   *[other] { $extra } fichiers non indexés
} sur la clé USB
diagnostics-check-parity-duplicates = Entrées PDB en double
diagnostics-check-parity-duplicates-detail = { $count ->
    [one] { $count } entrée de playlist PDB en double détectée
   *[other] { $count } entrées de playlists PDB en double détectées
}
diagnostics-check-parity-pdb-metadata = Complétude des métadonnées PDB
diagnostics-check-parity-pdb-metadata-detail = { $count ->
    [one] { $count } morceau PDB lié à une playlist
   *[other] { $count } morceaux PDB liés à une playlist
} sans les métadonnées requises par les lecteurs
diagnostics-check-parity-paths = Parité des chemins audio et d'analyse
diagnostics-check-parity-paths-detail = { $count ->
    [one] { $count } morceau lié à une playlist
   *[other] { $count } morceaux liés à une playlist
} avec des chemins audio ou d'analyse différents
diagnostics-check-parity-artwork = Parité de présence des pochettes
diagnostics-check-parity-artwork-detail = { $count ->
    [one] { $count } morceau lié à une playlist
   *[other] { $count } morceaux liés à une playlist
} avec une pochette dans une base mais pas dans l'autre
diagnostics-check-parity-dictionary = Résolution des id de dictionnaire PDB
diagnostics-check-parity-dictionary-detail = { $count ->
    [one] { $count } morceau lié à une playlist
   *[other] { $count } morceaux liés à une playlist
} avec des id de dictionnaire PDB requis non résolus
diagnostics-check-parity-edb-metadata = Complétude de la source eDB
diagnostics-check-parity-edb-metadata-detail = { $count ->
    [one] { $count } morceau eDB lié à une playlist
   *[other] { $count } morceaux eDB liés à une playlist
} sans les métadonnées utilisées par la comparaison de parité stricte
diagnostics-check-parity-rating-colour = Parité des notes et des couleurs
diagnostics-check-parity-rating-colour-detail = { $count ->
    [one] { $count } morceau lié à une playlist
   *[other] { $count } morceaux liés à une playlist
} avec une note ou une couleur différente dans la PDB et dans l'eDB (note différente : { $ratings }, couleur différente : { $colours })

diagnostics-progress-checking-pdb-integrity = USB : vérification de l'intégrité de la PDB
diagnostics-progress-checking-database-access = USB : vérification de l'accès aux bases de données
diagnostics-progress-checking-contents-integrity = USB : vérification de l'intégrité du contenu (base seule)
diagnostics-progress-checking-analysis-references = USB : vérification des références d'analyse
diagnostics-progress-checking-playlist-resolution = USB : vérification de la résolution des playlists
diagnostics-progress-checking-audio-files = USB : vérification de la présence des fichiers audio indexés
diagnostics-progress-comparing-playlist-sources = USB : comparaison des sources des playlists
diagnostics-progress-finalizing-parity-report = USB : finalisation du rapport de parité
diagnostics-progress-resolving-playlist = Résolution de la playlist { $playlist }

diagnostics-warning-missing-audio = fichier audio indexé absent de la clé USB : { $path }
diagnostics-warning-unindexed-audio = fichier audio non indexé : { $path }
diagnostics-warning-beat-grid-outdated = beatgrid dans un format obsolète : { $path }
diagnostics-warning-long-waveform = forme d'onde étirée par l'analyse jusqu'à 0.6.6 : { $path }
diagnostics-warning-shared-analysis = fichier d'analyse partagé : { $path } utilisé par { $tracks }
diagnostics-warning-bpm-key-mismatch = BPM/tonalité différents entre export.pdb et exportLibrary.db : { $path }
diagnostics-warning-string-misalignment = emplacement de chaîne mal aligné : { $row }
diagnostics-warning-history-only-track = morceau seulement dans l'historique : { $path } ({ $playlists })
diagnostics-warning-history-only-track-unknown = morceau seulement dans l'historique : { $path } (playlist d'historique inconnue)
diagnostics-warning-anlz-malformed = entrée d'analyse mal formée : { $path }
diagnostics-warning-anlz-malformed-empty = l'entrée d'analyse mal formée est vide : { $path }
diagnostics-warning-anlz-empty = le fichier d'analyse semble vide : { $path }
diagnostics-warning-anlz-unreadable = impossible de lire les métadonnées du fichier d'analyse { $path } : { $error }
diagnostics-warning-anlz-walk-failed = impossible de parcourir le dossier d'analyse USBANLZ : { $error }

## repair

repair-preview-ready = Aperçu de la réparation prêt ({ $ms } ms)
repair-apply-complete = Réparation appliquée : { $applied } appliquée(s), { $failed } en échec ({ $ms } ms)
repair-apply-complete-refreshed = Réparation appliquée : { $applied } appliquée(s), { $failed } en échec ({ $ms } ms). Diagnostics actualisés.

repair-overview-interrupted-export = Répare les dégâts laissés dans la base de données des lecteurs par un export interrompu.
repair-overview-track-count = Corrige les anciens lecteurs qui affichent 0 ou 1 morceau sur cette clé USB.
repair-overview-rekordbox-corrupted = Corrige les erreurs qui font signaler la clé USB comme corrompue par rekordbox.
repair-overview-header = Corrige un réglage de la base de données que certains lecteurs refusent.
repair-overview-string-alignment = Réécrit les textes de morceaux et d'albums qui peuvent figer les lecteurs pendant la navigation ou empêcher la liste des morceaux de s'ouvrir.
repair-overview-artist-names = Réécrit les noms d'artistes qui peuvent figer les lecteurs ou disparaître sur les lecteurs Denon / Numark.
repair-overview-history = Copie l'historique de lecture pour que les nouveaux lecteurs affichent le même historique que les anciens.
repair-overview-strict-parity = Rend chaque playlist identique sur les anciens et les nouveaux lecteurs.
repair-overview-duplicate-entries = Retire les morceaux listés deux fois par erreur dans une playlist.
repair-overview-footer-transaction = Corrige les données de playlists et de morceaux modifiées par une ancienne version de cette application, qui font planter rekordbox ou lui font perdre ses playlists lors de l'export suivant.
repair-overview-page-headers = Répare les pages que rekordbox a laissées à moitié écrites en plantant pendant un export.
repair-overview-damaged-tracks = Répare les morceaux dont rekordbox a endommagé le nom de fichier ou le titre en plantant, en gardant leurs playlists et leur historique.
repair-overview-damaged-playlists = Répare les playlists que rekordbox a endommagées en plantant : rend les noms endommagés et supprime une playlist à moitié écrite qui a pris la place d'une autre.
repair-overview-empty-duplicate-playlists = Supprime les copies vides de playlists qui portent le même nom qu'une playlist avec des morceaux.
repair-overview-relink = Reconnecte les morceaux aux fichiers audio renommés ou déplacés sur la clé USB.
repair-overview-unindexed = Place les fichiers audio qui ne sont dans aucune playlist dans une playlist que vous nommez.
repair-overview-duplicate-copies = Supprime les copies en trop de fichiers audio qui ne sont dans aucune playlist.
repair-overview-missing-audio = Retire les morceaux dont le fichier audio n'est plus sur la clé USB. L'historique de lecture les conserve.
repair-overview-empty-analysis = Reconstruit les formes d'onde et beatgrids manquantes.
repair-overview-app-analysis = Met à jour les formes d'onde, beatgrids et autres données d'analyse écrites par d'anciennes versions de l'application, pour que les lecteurs affichent la forme d'onde de chaque morceau et que le Beat Sync fonctionne.
repair-overview-bpm-key = Fait afficher le même BPM et la même tonalité à tous les lecteurs.
repair-overview-flac-seek = Ajoute les données dont les lecteurs ont besoin pour sauter à n'importe quel point des morceaux FLAC.
repair-overview-engine = Reconstruit la bibliothèque Denon / Numark et la tient à jour désormais. Les modifications faites sur ces lecteurs eux-mêmes sont remplacées.
repair-overview-other = Corrige d'autres erreurs dans la base de données des lecteurs.
repair-overview-needs-attention = Demande votre attention
repair-overview-not-fixable = { $count ->
    [one] { $count } problème ne peut pas être corrigé automatiquement
   *[other] { $count } problèmes ne peuvent pas être corrigés automatiquement
} — « Détails techniques » indique quoi faire.
repair-summary-no-issues = Aucun problème détecté.
repair-summary-nothing-fixable = Rien ici ne peut être corrigé automatiquement.
repair-summary = « Appliquer les corrections » répare { $count ->
    [one] { $count } problème
   *[other] { $count } problèmes
} sur cette clé USB. Les bases de données sont d'abord sauvegardées.

repair-fix-empty-analysis = Corriger les fichiers d'analyse vides
repair-fix-empty-analysis-description = Régénère les bundles DAT/EXT/2EX manquants ou vides quand l'audio source est retrouvable.
repair-fix-bpm-key = Corriger les écarts de BPM/tonalité
repair-fix-bpm-key-description = Donne à chaque ligne PDB et eDB d'un morceau le BPM et la tonalité de sa version la plus récente, pour que tous les lecteurs lisent le même BPM et la même tonalité : le BPM de la beatgrid de son fichier d'analyse, la tonalité de sa ligne la plus récente. Une clé USB exportée depuis plusieurs bibliothèques rekordbox a une ligne par bibliothèque pour un même fichier ; la plus récente est la dernière ligne de l'eDB, sinon celle de la PDB. Les fichiers d'analyse ne sont pas modifiés. Voir le Journal des événements pour les morceaux.
repair-fix-app-analysis = Mettre à jour les fichiers d'analyse de l'application
repair-fix-app-analysis-description = Morceaux dont le fichier d'analyse, écrit par une ancienne version de l'application, est obsolète : { $count }. Chacun reçoit ce qui lui manque : la forme d'onde de la bibliothèque (tirée de son audio quand le morceau n'est pas dans la bibliothèque) à la place d'une forme grossière ou étirée, son propre fichier quand il partage celui d'un autre morceau, le chemin du morceau (PPTH) d'après lequel les lecteurs le retrouvent, la beatgrid au format de rekordbox et l'index de recherche MP3 (PVBR). Un morceau de la bibliothèque dont la forme d'onde n'est pas encore tracée est d'abord analysé. Le BPM, les temps, les cues et ce qu'ont écrit les lecteurs sont conservés ; les fichiers d'analyse de rekordbox ne sont jamais modifiés. Voir le Journal des événements pour la liste.
repair-fix-flac-seek = Ajouter les données de recherche FLAC manquantes
repair-fix-flac-seek-description = Lit chaque FLAC de la clé USB et écrit un index de recherche (PVB2 dans le .EXT). Pour les morceaux longs, environ 98 % de ses entrées correspondent à celles de rekordbox ; les autres sont décalées d'une trame. Les beatgrids, les cues et les formes d'onde sont conservés. Les fichiers qui ne se lisent pas proprement sont ignorés et nommés dans le Journal des événements.
repair-fix-ec-conflict = Réparer le conflit de pointeur d'écriture de table
repair-fix-ec-conflict-description = Tables dont le pointeur empty_candidate vise une page déjà utilisée comme données par une autre table : { $count }. Le logiciel DJ vérifie cet invariant et signale « base de données corrompue » quand il trouve un conflit. La réparation réaffecte chaque empty_candidate en conflit à une page libre au-delà du fichier physique et met à jour next_unused_page dans l'en-tête.
repair-fix-truncated = Réparer la chaîne de table tronquée
repair-fix-truncated-description = Tables déclarant une dernière page qui n'existe pas physiquement dans le fichier : { $count } (une écriture d'agrandissement additive a été interrompue après l'avancement du pointeur de table mais avant l'écriture des nouvelles pages sur le disque). La réparation fait pointer les champs last/empty_candidate de la table vers la dernière page réellement écrite ; aucun contenu de page n'est modifié. C'est un prérequis structurel qui s'exécute toujours avant les autres réparations, y compris la mise en parité stricte.
repair-fix-torn-growth = Réparer les pages d'agrandissement déchirées (export interrompu)
repair-fix-torn-growth-description = L'export a été interrompu (p. ex. clé USB débranchée) pendant que rekordbox agrandissait la PDB : les pages empty_candidate des tables réservées comme prochain emplacement d'écriture ({ $count }) n'ont jamais reçu de données, laissant des octets parasites non nuls au lieu de la page vierge que l'écrivain exige avant réutilisation. Cette réparation remet chacune de ces pages à zéro et recalcule seqdb. Aucune chaîne de table, aucun index de pages ni aucun historique de transactions n'est touché. C'est un prérequis structurel qui s'exécute toujours avant les autres réparations, y compris la mise en parité stricte : les écritures additives de la parité peuvent agrandir le fichier, et la limite de troncature de fin de cette réparation deviendrait sinon obsolète et couperait des morceaux/playlists que la parité vient d'écrire.
repair-fix-torn-growth-description-tail = L'export a été interrompu (p. ex. clé USB débranchée) pendant que rekordbox agrandissait la PDB : les pages empty_candidate des tables réservées comme prochain emplacement d'écriture ({ $count }) n'ont jamais reçu de données, laissant des octets parasites non nuls au lieu de la page vierge que l'écrivain exige avant réutilisation, et le fichier est resté prolongé au-delà de next_unused_page avec un contenu jamais rempli. Cette réparation remet chacune de ces pages à zéro, tronque la fin jamais remplie et recalcule seqdb. Aucune chaîne de table, aucun index de pages ni aucun historique de transactions n'est touché. C'est un prérequis structurel qui s'exécute toujours avant les autres réparations, y compris la mise en parité stricte : les écritures additives de la parité peuvent agrandir le fichier, et la limite de troncature de fin de cette réparation deviendrait sinon obsolète et couperait des morceaux/playlists que la parité vient d'écrire.
repair-fix-t19 = Activer la page d'historique PDB
repair-fix-t19-description = La page tt=19 contient encore la ligne scellée unique d'une clé USB jamais exportée, les deux lignes fixes qu'écrivait une ancienne version, ou aucun nom de clé alors que la clé USB en a un, si bien que les anciens lecteurs comptent 0 ou 1 morceau. La réécrit sous forme de l'anneau d'une ligne par morceau que tient rekordbox, avec le nom de la clé USB, en gardant la ligne modèle, et marque la page comme active. Aucune autre page n'est modifiée.
repair-fix-track-alignment = Réparer l'alignement des chaînes des morceaux
repair-fix-track-alignment-description = Réencode { $count ->
    [one] { $count } ligne de morceau
   *[other] { $count } lignes de morceaux
} dont l'emplacement de chaîne UTF-16 ou ASCII longue (titre, nom de fichier, commentaire, etc.) ne commence pas à un décalage aligné sur 4 octets. Le matériel CDJ à base de MIPS exige des lectures alignées sur 4 octets pour cet en-tête de chaîne de 4 octets ; un emplacement mal aligné fige le lecteur ou provoque des erreurs de communication pendant la navigation, ou dès que la liste des morceaux s'ouvre.
repair-fix-album-alignment = Réparer l'alignement des chaînes des albums
repair-fix-album-alignment-description = Réencode { $count ->
    [one] { $count } ligne d'album
   *[other] { $count } lignes d'albums
} dont l'emplacement de nom UTF-16 ne commence pas à un décalage aligné sur 4 octets. Même type de problème que la réparation de l'alignement des chaînes des morceaux.
repair-fix-artist-layout = Réparer la disposition des noms d'artistes
repair-fix-artist-layout-description = Réencode { $count ->
    [one] { $count } ligne d'artiste
   *[other] { $count } lignes d'artistes
} comme rekordbox écrit les lignes d'artistes : un nom ASCII simple au décalage 10, un nom UTF-16 au décalage 12 aligné sur 4 octets. Les anciennes versions de l'application plaçaient tous les noms d'artistes à 12, ce que l'importation d'export.pdb par un lecteur Engine DJ lit comme une absence d'artiste.
repair-fix-header = Réparer le champ de compatibilité de l'en-tête PDB
repair-fix-header-description = Corrige uniquement les octets 0x10..0x14 de { $current } vers la valeur de compatibilité intégrée { $target }.
repair-fix-duplicate-copies = Supprimer les copies audio en double
repair-fix-duplicate-copies-description = Fichiers audio de la clé USB dans aucune playlist, chacun copie octet pour octet d'un morceau du même dossier : { $count }. Supprime chaque copie ; le morceau original garde ses playlists, ses cues et son analyse. Voir le Journal des événements pour la liste des fichiers.
repair-fix-unindexed = Ajouter l'audio non indexé à une playlist
repair-fix-unindexed-description = Fichiers audio de la clé USB dans aucune playlist (p. ex. après une restauration des bases de données depuis une sauvegarde) : { $count }. Ajoute chacun à la playlist USB nommée ci-dessous, en réutilisant ses fichiers d'analyse ; rien n'est copié ni supprimé. Voir le Journal des événements pour la liste des fichiers.
repair-fix-unindexed-name-label = Nom de la playlist
repair-fix-unindexed-default-name = Unindexed
repair-fix-relink = Reconnecter les fichiers audio déplacés
repair-fix-relink-description = Morceaux pointant vers un fichier absent alors que le même morceau est sur la clé USB sous un autre nom ou dans un autre dossier : { $count }. Fait pointer chacun vers le fichier trouvé ; il garde ses playlists, ses cues et son analyse. Voir le Journal des événements pour la liste.
repair-fix-missing-audio = Retirer les références audio manquantes
repair-fix-missing-audio-description = Fichiers audio qui ne sont plus sur la clé USB : { $count }. Retire leurs morceaux des deux bases de données (lignes de morceaux et de contenu, entrées de playlists). Les morceaux d'une playlist d'historique sont conservés. Voir le Journal des événements pour la liste des fichiers.
repair-fix-history-sync = Synchroniser les tables d'historique de l'eDB depuis la PDB
repair-fix-history-sync-description = Correction facultative : remplace les lignes history/history_content de l'eDB selon la correspondance actuelle des données d'historique de la PDB.
repair-fix-strict-parity = Mettre les données d'export en parité stricte
repair-fix-strict-parity-description = Rassemble les playlists en échec de l'eDB et de la PDB, fusionne les métadonnées des deux côtés et écrit une fois les deux bases de données avec les écrivains d'export. { $summary }.
repair-fix-strict-parity-description-failed = Rassemble les playlists en échec de l'eDB et de la PDB, fusionne les métadonnées des deux côtés et écrit une fois les deux bases de données avec les écrivains d'export. Échec de la simulation : { $error }.
repair-fix-strict-parity-pdb-missing = Cette clé USB a une eDB mais pas de PDB, les anciens lecteurs n'affichent donc aucune musique. Crée la PDB comme le fait « Initialiser la structure USB » et y écrit chaque playlist de l'eDB.
repair-fix-strict-parity-edb-missing = Cette clé USB a une PDB mais pas d'eDB, les nouveaux lecteurs et rekordbox n'affichent donc aucune musique. Crée l'eDB comme le fait « Initialiser la structure USB » et y écrit chaque playlist de la PDB.
repair-strict-parity-summary = { $playlists ->
    [one] { $playlists } playlist fusionnée
   *[other] { $playlists } playlists fusionnées
} ; eDB : { $edbadded ->
    [one] { $edbadded } ligne ajoutée
   *[other] { $edbadded } lignes ajoutées
}, { $edbchanged } modifiée(s) ; PDB : { $tracks ->
    [one] { $tracks } morceau ajouté
   *[other] { $tracks } morceaux ajoutés
}, { $trackschanged } modifié(s), { $pdbplaylists ->
    [one] { $pdbplaylists } playlist ajoutée
   *[other] { $pdbplaylists } playlists ajoutées
}, { $duplicates ->
    [one] { $duplicates } entrée en double retirée
   *[other] { $duplicates } entrées en double retirées
}
repair-strict-parity-rebuilt-pdb = PDB manquante reconstruite depuis l'eDB ; { $summary }
repair-strict-parity-rebuilt-edb = eDB manquante reconstruite depuis la PDB ; { $summary }
repair-strict-parity-flattened-copies = { $count ->
    [one] { $count } copie eDB de premier niveau d'une playlist rangée dans un dossier retirée
   *[other] { $count } copies eDB de premier niveau de playlists rangées dans des dossiers retirées
} ; { $summary }
repair-strict-parity-same-name-merged = playlists de même nom d'un dossier fusionnées ({ $count ->
    [one] { $count } playlist retirée
   *[other] { $count } playlists retirées
}) ; { $summary }
repair-fix-duplicate-entries = Retirer les entrées de playlists PDB en double
repair-fix-duplicate-entries-description = Retire { $count ->
    [one] { $count } ligne playlist_entries obsolète en double
   *[other] { $count } lignes playlist_entries obsolètes en double
} laissées sur des pages partagées par des exports répétés de la même playlist au fil du temps. Cela ne nettoie que les lignes en double déjà présentes dans la PDB — cela ne fusionne pas les playlists entre l'eDB et la PDB et ne réécrit pas l'export d'une autre façon. Pour cette réparation plus large, utilisez plutôt (ou en plus) « { repair-fix-strict-parity } ».
repair-fix-empty-duplicate-playlists = Supprimer les playlists vides en double
repair-fix-empty-duplicate-playlists-description = { $count ->
    [one] { $count } playlist vide de la PDB porte
   *[other] { $count } playlists vides de la PDB portent
} le nom d'une playlist avec des morceaux dans le même dossier. Supprime les copies vides de la PDB, et de l'eDB si elle les contient aussi ; les playlists avec des morceaux restent. S'exécute toujours avant la mise en parité stricte, qui associe les playlists par leur nom.
repair-fix-engine = Tenir à jour la bibliothèque Engine DJ
repair-fix-engine-description = Cette clé USB a une bibliothèque Engine DJ (Denon Prime, Numark Mixstream). Active « Écrire la bibliothèque Engine DJ » et la reconstruit maintenant à partir des données rekordbox de cette clé USB ; ensuite, elle est reconstruite chaque fois que l'application modifie une clé USB. L'historique de lecture est conservé, mais les modifications faites sur le lecteur lui-même (cues, boucles, playlists créées dessus) sont remplacées à chaque fois.

repair-unsupported-malformed-analysis = { $count ->
    [one] { $count } entrée USBANLZ mal formée
   *[other] { $count } entrées USBANLZ mal formées
}
repair-unsupported-malformed-analysis-reason = Elles indiquent une corruption du dossier d'analyse de la clé USB ou des fichiers égarés. Examinez les entrées du Journal des événements marquées « entrée d'analyse mal formée » et réexportez les morceaux concernés (ou reconstruisez les données d'analyse de la clé USB).
repair-unsupported-drive-layout = La clé USB est partitionnée ou formatée d'une façon que tous les lecteurs ne lisent pas
repair-unsupported-seek-data = { $count ->
    [one] { $count } morceau { $format }
   *[other] { $count } morceaux { $format }
} sans données de recherche qui ne peuvent pas être ajoutées
repair-unsupported-seek-data-reason = L'index de recherche ne peut pas être construit à partir de ces fichiers (le Journal des événements nomme chacun et explique pourquoi). Un fichier endommagé nécessite une copie intacte.
repair-unsupported-no-audio = { $count ->
    [one] { $count } fichier audio indexé manquant
   *[other] { $count } fichiers audio indexés manquants
} : la clé USB n'a pas d'audio sous Contents
repair-unsupported-no-audio-reason = Cela ressemble à une copie des bases de données sans la musique. Recopiez les fichiers audio au lieu de retirer tous les morceaux.
repair-unsupported-history-audio = { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} d'une playlist d'historique avec des fichiers audio manquants
repair-unsupported-history-audio-reason = Les morceaux d'une playlist d'historique ne sont pas retirés. Recopiez les fichiers sur la clé USB (le Journal des événements les liste).

repair-meta-supported = ✓ pris en charge
repair-meta-preview-only = ✗ aperçu seulement
repair-meta-destructive = destructif
repair-meta-safe = sans risque
repair-meta-writes = { $count ->
    [one] { $count } écriture
   *[other] { $count } écritures
}
repair-meta-deletes = { $count ->
    [one] { $count } suppression
   *[other] { $count } suppressions
}
repair-meta-always-applied = toujours appliqué
repair-meta-always-applied-tooltip = Toujours appliqué — nécessaire pour que les autres réparations se terminent sans risque


repair-fix-stale-btree = Réparer l'index B-tree sentinelle obsolète
repair-fix-footer-transaction = Réparer les pieds de page de la PDB
repair-fix-footer-transaction-description = Réécrit les champs de pied de page u5/num_rl sur { $count ->
    [one] { $count } page de données PDB
   *[other] { $count } pages de données PDB
} à partir des bits tranrf de la page, la forme qu'écrit rekordbox. D'anciennes versions de cette application les modifiaient, ce qui fait planter rekordbox 6 ou lui fait perdre ses playlists lors de l'export suivant.
repair-fix-page-headers = Réparer les en-têtes de pages PDB endommagés
repair-fix-page-headers-description = Rétablit les indicateurs de page et le nombre de lignes actives sur { $count ->
    [one] { $count } page de données PDB
   *[other] { $count } pages de données PDB
} à partir du pied de page des lignes. rekordbox laisse des pages ainsi lorsqu'il plante au milieu d'un export.
repair-fix-damaged-tracks = Fusionner les lignes de morceaux endommagées
repair-fix-damaged-tracks-description = { $count ->
    [one] { $count } ligne de morceau
   *[other] { $count } lignes de morceaux
} dont le nom de fichier a été écrasé par un plantage de rekordbox, à côté d'une ligne intacte pour le même fichier audio. Garde la ligne présente dans un historique, sinon la plus ancienne, y déplace les entrées de playlists et d'historique de l'autre ligne et supprime l'autre ligne, dans les deux bases.
repair-fix-rebuild-damaged-tracks = Reconstruire les lignes de morceaux endommagées
repair-fix-rebuild-damaged-tracks-description = { $count ->
    [one] { $count } ligne de morceau
   *[other] { $count } lignes de morceaux
} dont le nom de fichier, le titre ou un autre texte a été écrasé par un plantage de rekordbox, sans ligne intacte pour le même fichier audio. Rend à chacune son texte à partir du fichier audio de la clé USB auquel elle correspond encore, par sa taille et les caractères restés intacts, en gardant ses entrées de playlists et d'historique. Une ligne dont le fichier audio est introuvable est supprimée ou, si elle figure dans un historique, gardée sans les caractères endommagés.
repair-fix-damaged-playlists = Réparer les playlists endommagées
repair-fix-damaged-playlists-description = { $count ->
    [one] { $count } ligne de playlist endommagée
   *[other] { $count } lignes de playlists endommagées
} par un plantage de rekordbox. Rend les noms endommagés à partir de l'eDB, d'une sauvegarde ou des anciennes copies que rekordbox a laissées dans le PDB, et supprime une playlist à moitié écrite qui a pris l'identifiant d'une autre ; la réparation la rajoute depuis l'eDB quand l'eDB l'a.

progress-detail-count = { $done }/{ $total }
progress-detail-count-speed = { $done }/{ $total } · { $speed } Mo/s
progress-detail-count-playlist = { $done }/{ $total } · playlist { $index }/{ $count }
progress-detail-count-playlist-speed = { $done }/{ $total } · playlist { $index }/{ $count } · { $speed } Mo/s
progress-detail-bytes = { $done }/{ $total } Mo
progress-detail-bytes-speed = { $done }/{ $total } Mo · { $speed } Mo/s

repair-progress-baseline = USB : diagnostics de référence
repair-progress-parity = USB : préparation des indications de parité
repair-progress-collect = USB : recherche des réparations possibles
repair-progress-apply = USB : application des corrections choisies
repair-progress-backup = USB : sauvegarde des bases de données avant réparation
repair-progress-backup-file = USB : sauvegarde des bases de données avant réparation — { $file }
repair-progress-open = USB : ouverture des bases de données à réparer
repair-progress-save = USB : enregistrement des bases de données réparées
repair-progress-rerun = USB : nouveaux diagnostics après réparation
repair-progress-applied = USB : réparation appliquée
repair-progress-preview-ready = USB : aperçu de la réparation prêt
repair-progress-applying = USB : application de { $fix }
repair-progress-finished = USB : { $fix } terminé
repair-progress-applying-file = USB : application de { $fix } — { $file }

repair-warning-bpm-key-mismatch = Écart de BPM/tonalité : { $path } ({ $values })
repair-warning-shared-analysis = fichier d'analyse partagé : { $path } utilisé par { $tracks } ; gardé par { $owner }
repair-warning-shared-analysis-no-owner = fichier d'analyse partagé : { $path } utilisé par { $tracks } ; gardé par aucun morceau (chacun reçoit le sien)
repair-warning-seek-data-unfixable-replace = les données de recherche { $format } ne peuvent pas être ajoutées ({ $reason } ; remplacez-le par une copie intacte) : { $path }
repair-warning-seek-data-unfixable-report = les données de recherche { $format } ne peuvent pas être ajoutées ({ $reason } ; merci de signaler ce fichier (les détails du format, pas l'audio) pour que ses données de recherche puissent être prises en charge) : { $path }
repair-warning-missing-audio-scan-skipped = recherche d'audio manquant ignorée : le dossier Contents est absent ou vide (copie des bases seules)
repair-warning-history-audio-missing = fichier audio d'un morceau de l'historique manquant : { $path }
repair-warning-damaged-track = ligne de morceau endommagée par un plantage de rekordbox, sans copie intacte vers laquelle la fusionner : { $path }
repair-warning-damaged-track-removed = ligne de morceau endommagée par un plantage de rekordbox, fichier audio introuvable, supprimée : { $path }
repair-warning-damaged-track-kept = ligne de morceau endommagée par un plantage de rekordbox, fichier audio introuvable, gardée pour son historique sans les caractères endommagés : { $path }
repair-warning-duplicate-audio = fichier audio en double : { $path } (copie de { $original })
repair-warning-moved-audio = audio déplacé : { $old } -> { $new }
repair-warning-missing-audio = référence audio manquante : { $path }
repair-warning-empty-duplicate-playlist = playlist vide en double : { $name }
repair-warning-save-failed = Échec de l'enregistrement des bases de données réparées sur la clé USB : { $error }

repair-warning-duplicate-audio-removed = fichier audio en double supprimé : { $path }
repair-warning-beat-grid-length-unknown = beatgrid non reconstruite : durée du morceau inconnue pour { $path }
repair-warning-analysis-separated = propre fichier d'analyse pour { $track } : { $path }
repair-warning-app-analysis = fichier d'analyse obsolète : { $track } ({ $reasons })
repair-app-analysis-reason-shared = partage le fichier d'un autre morceau
repair-app-analysis-reason-repair-drawn = forme d'onde grossière d'une réparation antérieure
repair-app-analysis-reason-stretched = forme d'onde étirée par l'analyse jusqu'à 0.6.6
repair-app-analysis-reason-grid = beatgrid au format d'avant 0.3.7
repair-app-analysis-reason-track-path = sans chemin de morceau
repair-app-analysis-reason-mp3-seek = sans données de recherche MP3
repair-warning-app-analysis-unreadable = fichier d'analyse non mis à jour : impossible d'analyser l'audio de { $track }
repair-warning-app-analysis-library-failed = fichier d'analyse non mis à jour : la bibliothèque n'a pas pu analyser { $track }
repair-warning-app-analysis-write-failed = fichier d'analyse non mis à jour pour { $track } : { $error }
repair-warning-app-analysis-check-failed = fichiers d'analyse de l'application non vérifiés : { $error }
repair-warning-analysis-cancelled = non analysé, l'analyse a été annulée : { $track }
repair-warning-beat-grid-not-constant = beatgrid non réécrite (pas une grille à tempo constant) : { $path }
repair-warning-seek-data-skipped-replace = données de recherche { $format } non ajoutées ({ $reason }) : { $path } ; remplacez-le par une copie intacte
repair-warning-seek-data-skipped-report = données de recherche { $format } non ajoutées ({ $reason }) : { $path } ; merci de signaler ce fichier (les détails du format, pas l'audio) pour que ses données de recherche puissent être prises en charge
repair-warning-empty-analysis-no-track = réparation ignorée (analyse vide) : correspondance avec l'audio source introuvable pour { $path }
repair-warning-empty-analysis-audio-missing = réparation ignorée (analyse vide) : audio source manquant pour { $path } -> { $audio }
repair-warning-empty-analysis-unreadable = échec de la réparation (analyse vide) : impossible d'analyser l'audio source { $audio }
repair-warning-empty-analysis-length-unknown = échec de la réparation (analyse vide) : durée du morceau inconnue pour { $audio }
repair-warning-empty-analysis-write-failed = échec de la réparation (analyse vide) : { $path } : { $error }
repair-warning-relink-edb-unavailable = reconnexion de l'audio déplacé : eDB indisponible, seule la PDB a été mise à jour
repair-warning-unindexed-unreadable = audio non indexé ignoré, illisible : { $path } : { $error }
repair-warning-unindexed-no-analysis = l'audio non indexé n'a pas de fichiers d'analyse sur la clé USB : { $path }
repair-warning-unindexed-edb-unavailable = audio non indexé : eDB indisponible, seule la PDB a été mise à jour
repair-warning-missing-audio-edb-unavailable = réparation ignorée (audio manquant) : impossible d'ouvrir l'eDB en lecture-écriture

repair-log-applied = réparation appliquée : { $outcome }
repair-log-skipped = réparation ignorée : { $outcome }
repair-log-failed = échec de la réparation : { $outcome }
repair-outcome-applied = { $fix } : { $detail }
repair-outcome-nothing-to-apply = { $fix } : rien à appliquer
repair-outcome-not-selected = { $fix } : non sélectionnée
repair-outcome-pdb-unavailable = { $fix } : PDB indisponible
repair-outcome-failed = Échec de { $fix } : { $error }
repair-outcome-not-saved = { $outcome } (non enregistré sur la clé USB)
repair-outcome-not-run = { $fix } : non exécutée, les bases de données réparées n'ont pas été enregistrées
repair-outcome-read-failed = Échec de la lecture de la PDB, rien n'a été appliqué : { $error }
repair-outcome-save-failed = Échec de l'enregistrement des bases de données réparées sur la clé USB, les bases de la clé sont inchangées : { $error }
repair-outcome-detail-fixed-skipped = { $fixed } corrigé(s), { $skipped } ignoré(s)
repair-outcome-detail-fixed-skipped-failed = { $fixed } corrigé(s), { $skipped } ignoré(s), { $failed } en échec
repair-outcome-detail-duplicate-entries = { $count ->
    [one] { $count } entrée en double retirée
   *[other] { $count } entrées en double retirées
}
repair-outcome-detail-empty-duplicate-playlists = { $count ->
    [one] { $count } playlist vide supprimée
   *[other] { $count } playlists vides supprimées
}
repair-outcome-detail-relinked = { $count ->
    [one] { $count } morceau reconnecté
   *[other] { $count } morceaux reconnectés
}
repair-outcome-detail-unindexed = { $count ->
    [one] { $count } morceau ajouté
   *[other] { $count } morceaux ajoutés
} à « { $playlist } »
repair-outcome-detail-unindexed-notes = { $detail } ({ $notes })
repair-outcome-detail-unindexed-analysed = { $count } analysé(s) depuis l'audio
repair-outcome-detail-unindexed-without-analysis = { $count } sans fichiers d'analyse
repair-outcome-detail-duplicate-copies = { $count ->
    [one] { $count } fichier supprimé
   *[other] { $count } fichiers supprimés
}
repair-outcome-detail-bpm-key = { $pdb ->
    [one] { $pdb } ligne PDB mise à jour
   *[other] { $pdb } lignes PDB mises à jour
}, { $rows ->
    [one] { $rows } ligne eDB mise à jour
   *[other] { $rows } lignes eDB mises à jour
}
repair-outcome-detail-app-analysis = { $updated ->
    [one] { $updated } fichier d'analyse mis à jour
   *[other] { $updated } fichiers d'analyse mis à jour
}, { $separated } nouveaux, { $skipped } ignorés, { $failed } en échec
repair-outcome-detail-engine = réglage activé, reconstruite ({ $tracks ->
    [one] { $tracks } morceau
   *[other] { $tracks } morceaux
}, { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
})

## job

job-failed-unknown = Échec de l'opération
job-blocks-export = Attendez la fin de l'export.
job-blocks-usb-write = Attendez la fin de la modification de la clé USB.
job-blocks-usb-read = Attendez la fin du chargement de la clé USB.
job-blocks-diagnostics = Attendez la fin de la vérification ou de la réparation de la clé USB.
job-blocks-scan = Attendez la fin de la recherche de fichiers dans les dossiers.
job-blocks-transcode = Attendez la fin de la conversion, ou annulez-la.
job-blocks-analysis = Attendez la fin de l'analyse, ou annulez-la.
job-blocks-migration = Attendez la fin de la mise à niveau des données.
job-blocks-update = La mise à jour est en cours d'installation.
job-blocks-usb-order-edit = Enregistrez ou annulez d'abord la modification de l'ordre.
job-save-usb-track-metadata-edits-started = USB : enregistrement des détails des morceaux
job-save-usb-track-metadata-edits-completed = USB : détails des morceaux enregistrés
job-save-usb-track-analysis-edits-started = USB : enregistrement des modifications de cues
job-save-usb-track-analysis-edits-completed = USB : modifications de cues enregistrées
job-validate-usb-root-started = USB : validation de la racine
job-validate-usb-root-completed = USB : validation de la racine terminée
job-validate-usb-root-progress = USB : vérification des dossiers et des droits
job-migrate-started = Mise à niveau des données d'analyse
job-migrate-completed = Données d'analyse mises à niveau
job-fetch-usb-playlists-started = USB : lecture des playlists
job-fetch-usb-playlists-completed = USB : lecture des playlists terminée
job-fetch-usb-histories-started = USB : lecture de l'historique
job-fetch-usb-histories-completed = USB : lecture de l'historique terminée
job-get-usb-player-menu-config-started = USB : chargement du menu du lecteur
job-get-usb-player-menu-config-completed = USB : menu du lecteur chargé
job-get-usb-player-menu-config-progress = USB : lecture de la configuration du menu
job-update-usb-player-menu-config-started = USB : mise à jour du menu du lecteur
job-update-usb-player-menu-config-completed = USB : menu du lecteur mis à jour
job-update-usb-player-menu-config-progress = USB : écriture de la configuration du menu
job-get-usb-background-color-started = USB : chargement de la couleur de fond
job-get-usb-background-color-completed = USB : couleur de fond chargée
job-get-usb-background-color-progress = USB : lecture de la couleur de fond
job-set-usb-background-color-started = USB : réglage de la couleur de fond
job-set-usb-background-color-completed = USB : couleur de fond réglée
job-set-usb-background-color-progress = USB : écriture de la couleur de fond
job-remove-usb-playlist-started = USB : retrait de la playlist
job-remove-usb-playlist-completed = USB : retrait de la playlist terminé
job-commit-usb-order-edit-started = USB : enregistrement du nouvel ordre
job-commit-usb-order-edit-completed = USB : ordre enregistré
job-export-to-usb-started = USB : export de la playlist
job-export-playlists-to-usb-started = USB : export des playlists
job-export-to-usb-completed = USB : export terminé
job-run-usb-diagnostics-started = USB : diagnostics en cours
job-run-usb-diagnostics-completed = USB : diagnostics terminés
job-run-usb-parity-report-started = USB : rapport de parité en cours
job-run-usb-parity-report-completed = USB : rapport de parité terminé
job-repair-usb-diagnostics-started = USB : préparation des corrections
job-repair-usb-diagnostics-completed = USB : préparation des corrections terminée
job-initialize-usb-started = USB : initialisation de la structure
job-initialize-usb-completed = USB : initialisation terminée
job-initialize-usb-progress = USB : création des dossiers External library
job-scan-library-started = Analyse de la médiathèque
job-scan-library-progress = Recherche des fichiers audio
job-scan-library-completed = Analyse de la bibliothèque terminée
job-scan-library-completed-counts = Analyse de la bibliothèque terminée : { $indexed } indexé(s), { $updated } mis à jour, { $removed } retiré(s)
job-scan-rekordbox-db-progress = Lecture de la base de données de la bibliothèque de bureau
job-scan-rekordbox-db-completed = Importation de la bibliothèque de bureau terminée
job-scan-rekordbox-db-completed-counts = Importation de la bibliothèque de bureau terminée : { $indexed } indexé(s), { $updated } mis à jour
job-scan-mixxx-db-progress = Lecture de la base de données de la bibliothèque Mixxx
job-scan-mixxx-db-completed = Importation de la bibliothèque Mixxx terminée
job-scan-mixxx-db-completed-counts = Importation de la bibliothèque Mixxx terminée : { $indexed } indexé(s), { $updated } mis à jour
job-transcode-tracks-started = Conversion des morceaux sélectionnés
job-transcode-tracks-progress = Conversion de { $file }
job-transcode-tracks-completed = Conversion terminée : { $converted } converti(s), { $skipped } ignoré(s), { $failed } en échec
job-transcode-tracks-cancelled = Conversion annulée : { $converted } converti(s)
job-install-update-started = Recherche de mise à jour...
job-install-update-downloading = Téléchargement de la mise à jour { $version }...
job-install-update-waiting = Mise à jour { $version } téléchargée, attente de la fin de la tâche en cours...
job-install-update-installing = Installation de la mise à jour { $version } et redémarrage...
job-install-update-failed = Échec de la mise à jour : { $error }

## migration

migration-usb-devices = Importation des clés USB depuis les anciens paramètres
migration-fingerprints = Calcul des empreintes des morceaux
migration-merge-usb-tracks = Fusion des morceaux USB en double
migration-beat-grids = Mise à niveau des beatgrids en cache
migration-mp3-seek = Ajout des données de recherche MP3
migration-long-track-waveforms = Nouveau tracé des formes d’onde des longs morceaux
migration-stratum-v1-analysis = Marquage pour analyse des morceaux dont l’analyse du BPM est ancienne
migration-forget-removed-usb-drives = Oubli des morceaux des clés USB retirées de la liste
migration-split-analysis-engine = Séparation du réglage du moteur d’analyse en BPM et tonalité

migration-warning-failed = Échec de { $migration } ({ $error }) ; nouvelle tentative au prochain lancement
migration-warning-startup-failed = Échec des mises à niveau de données au démarrage ({ $error }) ; nouvelle tentative au prochain lancement
migration-failed = Échec de la mise à niveau des données : { $error }

## transcode

transcode-choice-lossless = { $format } (sans perte)
transcode-choice-mp3 = MP3 320 kbps
transcode-summary-done = Conversion terminée : { $parts }
transcode-summary-cancelled = Conversion annulée : { $parts }
transcode-summary-converted = { $count ->
    [one] { $count } morceau converti
   *[other] { $count } morceaux convertis
}
transcode-summary-skipped = { $count } ignoré(s)
transcode-summary-failed = { $count } en échec
transcode-removed = { $count ->
    [one] { $count } transcodage supprimé
   *[other] { $count } transcodages supprimés
}
transcode-tooltip = Transcodage { $format } · { $size }
transcode-tooltip-outdated = { $about } — obsolète : le fichier source a changé depuis. Convertissez à nouveau pour le mettre à jour
transcode-export-source = source : { $format }
transcode-export-exported = Transcodage { $format } · { $size }, exporté sur cette clé USB — { $source }
transcode-export-convert = Converti en { $format } à l'export — { $source }
transcode-export-refused = { $file } — le { $format } ne peut pas aller sur cette clé USB : seuls les fichiers sans perte, OGG et Opus peuvent être convertis

transcode-error-folder-in-source = le dossier des transcodages ne peut pas être dans un dossier source : l'analyse listerait les transcodages comme des morceaux

transcode-skipped-not-found = { $file } : fichier introuvable
transcode-skipped-already-format = { $file } : déjà dans ce format
transcode-skipped-not-lossless = { $file } : ni sans perte, ni OGG, ni Opus
transcode-skipped-up-to-date = { $file } : a déjà un transcodage à jour
transcode-failed = { $file } : { $reason }

## tracklist-export

tracklist-export-done = Tracklist exportée : { $history }
tracklist-export-cancelled = Export de la tracklist annulé
tracklist-export-dialog-title = Enregistrer la tracklist
tracklist-export-dialog-filter-text = Texte

## analysis

analysis-submitted = { $count ->
    [one] Analyse de { $count } morceau
   *[other] Analyse de { $count } morceaux
}
analysis-submitted-already = { $count ->
    [one] { $count } morceau déjà en cours d'analyse
   *[other] { $count } morceaux déjà en cours d'analyse
}
analysis-submitted-some-already = { $count ->
    [one] Analyse de { $count } morceau
   *[other] Analyse de { $count } morceaux
}, { $already } déjà en cours
analysis-submitted-none = Aucun morceau n'a besoin d'analyse
analysis-waiting-tooltip = En attente de son tour d'analyse
analysis-started = Analyse des morceaux sélectionnés
analysis-progress = Analyse de { $file }
analysis-job-track-failed = Échec de l'analyse du morceau : { $track } - { $error }
analysis-job-track-failed-unknown = erreur d'analyse inconnue
analysis-finished = Analyse terminée : { $analyzed } analysé(s), { $failed } en échec
analysis-finished-incomplete = Analyse terminée : { $analyzed } analysé(s), { $failed } en échec, { $incomplete } incomplet(s)
analysis-log = Analyse : { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} en { $elapsed } ({ $analyzed } analysé(s), { $failed } en échec)
analysis-log-incomplete = Analyse : { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} en { $elapsed } ({ $analyzed } analysé(s), { $failed } en échec, { $incomplete } incomplet(s))

analysis-cancelled = Analyse annulée : { $analyzed } sur { $total ->
    [one] { $total } morceau
   *[other] { $total } morceaux
} analysé(s)
analysis-no-eligible-tracks = Aucun morceau à analyser
analysis-track-incomplete = { $path } : { $reasons }
analysis-incomplete-no-bpm = aucun BPM détecté
analysis-incomplete-no-duration = aucune durée
analysis-incomplete-no-waveform = aucune forme d'onde
analysis-track-failed = { $path } : { $error }
analysis-warning-mp3-seek-skipped-replace = données de recherche MP3 non ajoutées ({ $reason }) : { $path } ; remplacez-le par une copie intacte
analysis-warning-mp3-seek-skipped-report = données de recherche MP3 non ajoutées ({ $reason }) : { $path } ; merci de signaler ce fichier (les détails du format, pas l'audio) pour que ses données de recherche puissent être prises en charge

## error

error-track-not-found = morceau introuvable : { $id }
error-playlist-not-found = playlist introuvable : { $id }
error-playlist-folder-not-found = dossier de playlists introuvable : { $id }
error-track-not-found-anywhere = morceau introuvable dans la Bibliothèque ou sur la clé USB sélectionnée
error-usb-playlist-not-found = playlist USB introuvable : { $playlist }
error-usb-history-not-found = historique USB introuvable : { $history }
error-remove-failed = Échec de la suppression : { $error }
error-file-not-found = Fichier introuvable : { $path }
error-audio-file-not-found = fichier audio introuvable : { $path }
error-audio-path-not-file = le chemin audio n'est pas un fichier : { $path }
error-usb-edb-unreadable = base de données de la bibliothèque USB (exportLibrary.db) introuvable ou illisible
error-usb-analysis-path-not-found = chemin d'analyse USB introuvable : { $path }
error-usb-analysis-unreadable = bundle d'analyse USB illisible : { $path }
error-usb-analysis-empty = le bundle d'analyse USB est vide : { $path }
error-usb-analysis-missing = bundle d'analyse USB manquant : { $path }
error-usb-root-missing = le usbRoot demandé n'existe pas : { $path }
error-data-upgrade-running = Mise à niveau des données en cours, réessayez dans un instant

## export

export-button = Exporter vers la clé USB : { $dir }
export-button-tooltip = Exporter la playlist actuelle vers la clé USB sélectionnée
export-button-append = Ajouter à ({ $playlist }) sur la clé USB : ({ $dir })
export-button-append-no-dir = Ajouter à ({ $playlist }) sur la clé USB
export-button-append-tooltip = Ajouter les morceaux de la playlist actuelle à la playlist USB existante « { $playlist } »
export-button-no-usb = Choisissez d'abord une clé USB
export-button-no-usb-tooltip = Choisissez d'abord un dossier USB valide
export-mark-on-usb = Sur cette clé USB
export-mark-exported = Exportée sur clé USB
export-space-to-copy = { $count } sur { $total ->
    [one] { $total } morceau
   *[other] { $total } morceaux
} à copier
export-space-all-on-usb = Tous les morceaux sont sur la clé USB
export-format-convert = { $count ->
    [one] { $count } morceau sera converti à l'export
   *[other] { $count } morceaux seront convertis à l'export
}
export-format-convert-to = { $count ->
    [one] { $count } morceau sera converti en { $format } à l'export
   *[other] { $count } morceaux seront convertis en { $format } à l'export
}
export-format-refused = { $count ->
    [one] { $count } morceau ne peut pas aller sur cette clé USB ; l'export est bloqué
   *[other] { $count } morceaux ne peuvent pas aller sur cette clé USB ; l'export est bloqué
}
export-format-refused-kind = { $count ->
    [one] { $count } morceau { $format } ne peut pas aller sur cette clé USB ; l'export est bloqué
   *[other] { $count } morceaux { $format } ne peuvent pas aller sur cette clé USB ; l'export est bloqué
}
export-space-creates-database = la clé USB n'a pas encore de { $database } : l'export la crée
export-space-free = { $size } libres sur la clé USB
export-space-does-not-fit = Ne tient pas : { $line }
export-complete = Export terminé : { $playlist } - { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}, { $skipped } ignoré(s) [synchro : { $mode }]
export-sync-mirror = miroir
export-sync-additive = additif
batch-export-complete = Export terminé : { $playlists ->
    [one] { $playlists } playlist
   *[other] { $playlists } playlists
} - { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}, { $skipped } ignoré(s) [synchro : { $mode }]
batch-export-error-none-selected = Choisissez au moins une playlist à exporter.
batch-export-error-not-exportable = { $playlist } ne peut pas être exportée : { $reason }
batch-export-error-order-needs-all = Garder l'ordre des playlists de l'application nécessite de choisir toutes les playlists.
batch-export-reason-empty = aucun morceau
batch-export-reason-none-playable = aucun de ses morceaux n'est dans un format que lisent les lecteurs
batch-export-reason-source-missing = son dossier de musique n'est pas connecté
batch-export-reason-not-analyzed = certains morceaux ne sont pas encore analysés
batch-export-reason-formats = certains morceaux sont dans des formats que cette clé USB n'accepte pas
batch-export-analyze-missing-tooltip = Ajouter la forme d'onde, le BPM, la tonalité et le premier temps manquants aux morceaux non analysés des playlists sélectionnées, pour que les playlists en attente d'analyse puissent être exportées ; un BPM ou une tonalité modifiés ou importés sont conservés
batch-export-copy = { $count ->
    [one] { $count } morceau · { $size }
   *[other] { $count } morceaux · { $size }
}
batch-export-copy-none = Rien de nouveau
batch-export-never = Jamais
batch-export-last-exported = { $date } · { $drive }
batch-export-last-exported-no-drive = { $date }
batch-export-status-new = Pas encore sur la clé
batch-export-status-on-stick = Sur la clé, à jour
batch-export-status-changed = Sur la clé, modifiée depuis
batch-export-force-track-order-tooltip = Les playlists déjà sur la clé USB reçoivent les morceaux de l'application dans l'ordre de l'application. Les morceaux présents seulement sur la clé quittent ces playlists ; leurs fichiers restent. Sans cette option, les nouveaux morceaux vont à la fin et les autres gardent leur ordre.
batch-export-force-track-order-mirror-tooltip = L'export miroir écrit toujours l'ordre des morceaux de l'application.
export-refused-no-space = Export bloqué : pas assez d'espace libre sur la clé USB — il faut environ { $required }, { $available } libres.
export-refused-formats = Export bloqué : { $count ->
    [one] { $count } morceau est
   *[other] { $count } morceaux sont
} dans un format que les formats de cette clé USB n'acceptent pas, et seuls les fichiers sans perte, OGG et Opus peuvent être convertis. Voir le Journal des événements pour les détails.
export-refused-analysis-files = Export bloqué : les fichiers d'analyse de { $missing } sur { $total ->
    [one] { $total } morceau
   *[other] { $total } morceaux
} sont manquants ou endommagés. Sélectionnez-les et utilisez « Réanalyser ».
export-refused-not-analyzed = Export bloqué : { $missing } sur { $total ->
    [one] { $total } morceau
   *[other] { $total } morceaux
} pas encore analysé(s). Utilisez « Analyser les morceaux non analysés ».
export-refused-read-only = La clé USB est en lecture seule. Remontez-la en lecture-écriture avant l'export.
export-refused-manifest = Export bloqué : échec de la vérification des invariants du manifeste.
export-refused-manifest-after-prune = Export terminé avec des erreurs : le nettoyage des fichiers obsolètes a supprimé un fichier que la playlist exportée utilise encore — les bases de données USB ont été écrites ; restaurez la sauvegarde faite avant l'export.


export-progress-preparing = USB : préparation de l'export
export-progress-finalizing-export-metadata = USB : finalisation des métadonnées d'export
export-progress-writing-engine-dj-library = USB : écriture de la bibliothèque Engine DJ
export-progress-waiting-for-writes = USB : attente de la fin de l'écriture des fichiers
export-progress-copying = USB : copie de { $file }
export-progress-batch-playlist = { $step } pour { $playlist }
export-progress-converting = USB : conversion de { $file }

export-error-busy = Un autre export ou une autre modification de la clé USB est encore en cours. Exportez à nouveau une fois terminé.
export-error-usb-missing = La racine USB n'existe pas : { $path }
export-error-no-tracks = la playlist n'a aucun morceau à exporter
export-error-none-playable = aucun des morceaux de la playlist n'est dans un format que lisent les lecteurs (MP3, AAC, WAV, AIFF, FLAC, ALAC)
export-error-none-convertible = aucun des morceaux de la playlist n'a pu être converti pour cette clé USB
export-error-formats = Export bloqué : { $count ->
    [one] { $count } morceau est
   *[other] { $count } morceaux sont
} dans un format que les formats de cette clé USB n'acceptent pas, et seuls les fichiers sans perte, OGG et Opus peuvent être convertis : { $tracks }
export-error-not-analyzed = Export bloqué : il manque à { $missing }/{ $total } morceaux de la playlist l'analyse requise (forme d'onde, BPM, durée, fichiers DAT/EXT) ; lancez d'abord l'analyse
export-error-source-missing = Export bloqué : le dossier source est introuvable : { $folder }. Relocalisez-le ou retirez-le d'abord.
export-error-edb-missing = eDB introuvable à { $path }

export-warning-unplayable-format = non exporté, les lecteurs ne lisent pas son format : { $path }
export-warning-source-missing = fichier source manquant : { $path }
export-warning-artwork-missing-after = avertissement de vérification de l'export : pochette manquante pour le morceau « { $track } » : { $path }
export-warning-analysis-missing-after = avertissement de vérification de l'export : analyse manquante ou endommagée pour le morceau « { $track } » : { $path }
export-warning-transcode-no-analysis = aucune analyse pour le transcodage de { $path }
export-warning-transcode-failed = non exporté, sa conversion a échoué : { $path } : { $reason }
export-warning-transcode-failed-unknown = non exporté, sa conversion a échoué : { $path }
export-warning-artwork-source-missing = pochette manquante : { $path }
export-warning-artwork-unreadable = échec du décodage de la pochette pour { $path } : { $error }
export-warning-artwork-write-failed = échec de l'encodage de la pochette pour { $path } : { $error }
export-warning-analysis-not-made = bundle d'analyse manquant pour le morceau { $track } (pas de chemin DAT)
export-warning-analysis-files-missing = bundle d'analyse manquant pour le morceau { $track } : { $path }

export-engine-written = Bibliothèque Engine DJ écrite (morceaux : { $tracks }, analysés : { $analyzed }, playlists : { $playlists })
export-engine-not-written = Bibliothèque Engine DJ non écrite : { $error }
export-engine-tracks-without-analysis = { $missing } sur { $total ->
    [one] { $total } morceau
   *[other] { $total } morceaux
} sans beatgrid ni forme d'onde utilisable sur la clé USB ; le lecteur Engine les analyse lui-même au chargement

## track-list

track-list-summary-size = { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} · { $size }
track-list-total-time = Durée totale : { $duration }
track-list-total-time-unknown = Durée totale : { $duration } ({ $count ->
    [one] { $count } sans durée
   *[other] { $count } sans durée
})

## format

format-decimal-separator = ,

format-track-count = { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}

format-size-b = { NUMBER($size) } o
format-size-kb = { NUMBER($size) } Ko
format-size-mb = { NUMBER($size) } Mo
format-size-gb = { NUMBER($size) } Go

## update

update-available = La version { $version } est disponible.
update-available-package-manager = La version { $version } est disponible. Mettez-la à jour avec votre gestionnaire de paquets.
update-check-failed = Impossible de rechercher des mises à jour.
update-up-to-date = À jour.
update-banner-critical = Mise à jour critique disponible : { $version }
update-banner-feature = Nouvelles fonctionnalités disponibles : { $version }

## settings

settings-language-unknown = Langue inconnue : { $language }
settings-warning-language-unavailable = La langue enregistrée { $language } n'est pas disponible ; utilisation de l'anglais
settings-language-name = Français
settings-language-failed = Impossible de changer la langue : { $error }
settings-transcode-folder-dialog-title = Choisir le dossier des transcodages
settings-transcode-folder-failed = Dossier des transcodages : { $error }
settings-sync-mode-mirror-saved = Mode de synchronisation de l'export : miroir (correspondance exacte)
settings-sync-mode-additive-saved = Mode de synchronisation de l'export : additif
settings-sync-mode-additive-locked = Mode de synchronisation de l'export : additif — « { $playlist } » existe déjà sur la clé USB, l'ordre de ses morceaux est donc verrouillé ici (ordre actuel conservé).
settings-sync-mode-additive-locked-sort-failed = Mode de synchronisation de l'export : additif — « { $playlist } » existe déjà sur la clé USB, l'ordre de ses morceaux est donc verrouillé ici (le tri actuel n'a pas pu être enregistré avant).

## library

library-source-folders-dialog-title = Choisir les dossiers sources de la médiathèque

library-sources-filter-status = Filtres de sources : { $enabled }/{ $total } activés

library-folders-indexed = Dossiers sources : { $folders } | { $count ->
    [one] { $count } morceau indexé
   *[other] { $count } morceaux indexés
}

library-folders-added-index-failed = { $count ->
    [one] { $count } dossier source ajouté
   *[other] { $count } dossiers sources ajoutés
}, mais l'indexation a échoué : { $error }

library-folders-unchanged = Dossiers sources inchangés : { $folders }

library-folder-removed = Dossiers sources : { $folders } | { $count ->
    [one] { $count } morceau retiré
   *[other] { $count } morceaux retirés
} de la bibliothèque

library-relocated = Source relocalisée : { $count ->
    [one] { $count } chemin de morceau mis à jour
   *[other] { $count } chemins de morceaux mis à jour
}

library-relocated-unresolved = Source relocalisée : { $count ->
    [one] { $count } chemin de morceau mis à jour
   *[other] { $count } chemins de morceaux mis à jour
} | { $unresolved ->
    [one] { $unresolved } morceau toujours non résolu
   *[other] { $unresolved } morceaux toujours non résolus
}

library-relocated-and-indexed = { $relocated } | { $count ->
    [one] { $count } nouveau morceau indexé
   *[other] { $count } nouveaux morceaux indexés
}

library-relocated-index-failed = { $relocated } | l'indexation du nouveau dossier a échoué : { $error }

library-import-done-rekordbox = Importation de la bibliothèque de bureau terminée : { $new } nouveau(x), { $updated } mis à jour

library-import-done-rekordbox-not-found = Importation de la bibliothèque de bureau terminée : { $new } nouveau(x), { $updated } mis à jour | { $count ->
    [one] { $count } fichier introuvable
   *[other] { $count } fichiers introuvables
} (voir le journal des événements)

library-import-done-mixxx = Importation de la bibliothèque Mixxx terminée : { $new } nouveau(x), { $updated } mis à jour

library-import-done-mixxx-not-found = Importation de la bibliothèque Mixxx terminée : { $new } nouveau(x), { $updated } mis à jour | { $count ->
    [one] { $count } fichier introuvable
   *[other] { $count } fichiers introuvables
} (voir le journal des événements)

library-import-duplicate-files = { $count ->
    [one] { $count } fichier en double
   *[other] { $count } fichiers en double
} (voir le journal des événements)

library-error-no-source-folder = Ajoutez un dossier source avant l'analyse
library-error-no-source-enabled = Activez au moins un dossier source avant l'analyse
library-error-source-file-missing = le fichier source n'existe pas : { $path }
library-error-folder-not-configured = dossier source non configuré : { $folder }
library-error-relocate-same = Le nouveau dossier doit être différent de l'ancien
library-error-relocate-missing = Le nouveau dossier n'existe pas ou n'est pas un dossier : { $folder }
library-error-mixxx-not-found = Bibliothèque Mixxx introuvable
library-error-mixxx-not-found-at = Bibliothèque Mixxx introuvable : { $path }
library-error-mixxx-open = impossible d'ouvrir la bibliothèque Mixxx : { $error }
library-error-mixxx-tables = Bibliothèque Mixxx ouverte, mais tables library/track_locations introuvables
library-error-mixxx-query = Échec de la requête sur la bibliothèque Mixxx : { $error }
library-error-mixxx-row = Erreur de ligne dans la bibliothèque Mixxx : { $error }
library-error-mixxx-playlist-query = Échec de la requête sur les playlists Mixxx : { $error }
library-error-mixxx-playlist-not-found = Playlist Mixxx introuvable : { $id }
library-error-rekordbox-not-found = master.db introuvable
library-error-rekordbox-not-found-at = master.db introuvable : { $path }
library-error-rekordbox-open = impossible d'ouvrir master.db : { $error }
library-error-rekordbox-key = échec de la clé de master.db : { $error }
library-error-rekordbox-tables = master.db ouvert, mais table djmdContent introuvable (mauvaise clé ou version)
library-error-rekordbox-query = Échec de la requête sur master.db : { $error }
library-error-rekordbox-row = Erreur de ligne dans master.db : { $error }
library-error-rekordbox-playlist-not-found = Playlist rekordbox introuvable : { $id }

library-warning-duplicate-file = Même fichier à deux endroits : { $duplicate } est identique à { $kept }. La bibliothèque en garde un ; supprimez la copie en trop.
library-warning-source-folders-missing = { $count ->
    [one] { $count } dossier source
   *[other] { $count } dossiers sources
} introuvable(s) ou qui ne sont pas des dossiers : { $folders }. { $count ->
    [one] Relocalisez-le ou retirez-le.
   *[other] Relocalisez-les ou retirez-les.
}

## scan

scan-warning-analysis-not-found = fichier d'analyse introuvable : { $path }
scan-warning-artwork-not-found = pochette introuvable : { $path }
scan-warning-track-file-not-found = fichier du morceau introuvable, ignoré : { $path }
scan-warning-cover-not-found = fichier de pochette introuvable : { $path } (pour { $track })
scan-warning-artwork-copy-failed = échec de la copie de la pochette : { $path } : { $error }
scan-note-cues-imported = cues importés pour { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}
scan-note-hot-cues-imported = hot cues importés pour { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}
scan-note-cues-kept-local = valeurs de la bibliothèque conservées pour { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} dont les cues, le BPM, la tonalité ou la grille de temps diffèrent dans { $library }
scan-note-unsupported-skipped = { $count ->
    [one] { $count } morceau ignoré
   *[other] { $count } morceaux ignorés
} : fichier audio non pris en charge
scan-warning-analysis-missing = { $count ->
    [one] { $count } fichier d'analyse introuvable
   *[other] { $count } fichiers d'analyse introuvables
} sur le disque
scan-warning-artwork-missing = { $count ->
    [one] { $count } fichier de pochette introuvable
   *[other] { $count } fichiers de pochette introuvables
} ou copie échouée
scan-warning-cover-missing = { $count ->
    [one] { $count } fichier de couverture introuvable
   *[other] { $count } fichiers de couverture introuvables
} ou copie échouée
scan-note-playlist-duplicates = { $count ->
    [one] { $count } morceau répété
   *[other] { $count } morceaux répétés
} dans « { $playlist } » ajouté(s) une seule fois
scan-warning-playlist-tracks-deleted = { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} de « { $playlist } » ne sont plus dans la bibliothèque { $library }
scan-warning-playlist-tracks-unplayable = { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} de « { $playlist } » laissé(s) de côté : les lecteurs ne lisent pas l'AAC protégé et l'application ne peut pas le convertir

## playlist

playlist-created = Playlist créée : { $name }
playlist-deleted = Playlist supprimée : { $name }
playlist-folder-created = Dossier créé : { $name }
playlist-folder-deleted = Dossier supprimé : { $name }
playlist-folder-delete-confirm = Supprimer le dossier « { $name } » ?
playlist-folder-delete-confirm-items = Supprimer le dossier « { $name } » ?

    Son contenu remonte d'un niveau. Aucune playlist n'est supprimée.
playlist-folder-collapse = Replier
playlist-folder-expand = Déplier

playlist-delete-confirm = Supprimer « { $name } » ?

    Cela supprime la playlist de l'application et ses liens playlist-morceaux dans l'application.
playlist-delete-confirm-exported = Supprimer « { $name } » ?

    Cela supprime la playlist de l'application et ses liens playlist-morceaux dans l'application.

    Cette playlist a déjà été exportée. Il est facile de la recréer en important les playlists depuis la clé USB.

playlist-export-last = Dernier export le { $date } vers { $drive } ({ $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}).
playlist-export-last-no-drive = Dernier export le { $date } ({ $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}).
playlist-export-last-unknown-count = Dernier export le { $date } vers { $drive } (nombre de morceaux inconnu).
playlist-export-last-no-drive-unknown-count = Dernier export le { $date } (nombre de morceaux inconnu).
playlist-export-on-usb = Sur cette clé USB.
playlist-export-not-on-usb = Pas sur cette clé USB.
playlist-export-usb-and-last = { $usb } { $last }

playlist-tracks-added = { $count ->
    [one] { $count } morceau ajouté
   *[other] { $count } morceaux ajoutés
} ({ $skipped } ignoré(s)) à { $name }
playlist-tracks-added-unplayable = { $count ->
    [one] { $count } morceau ajouté
   *[other] { $count } morceaux ajoutés
} ({ $skipped } ignoré(s)) à { $name }. Non ajouté(s) : { $unplayable ->
    [one] { $unplayable } morceau
   *[other] { $unplayable } morceaux
} en AAC protégé, que les lecteurs ne lisent pas et que l'application ne peut pas convertir

playlist-tracks-removed = { $count ->
    [one] { $count } morceau retiré
   *[other] { $count } morceaux retirés
} de { $name }

playlist-analyze-missing = Analyser les morceaux non analysés ({ $count })
playlist-analyze-missing-tooltip = Ajouter la forme d'onde, le BPM, la tonalité et le premier temps manquants aux morceaux non analysés de cette playlist ; un BPM ou une tonalité modifiés ou importés sont conservés
playlist-analyze-missing-none-tooltip = Aucun morceau de cette playlist n'a besoin d'analyse

playlist-track-source-usb-not-connected = Clé USB non branchée : { $path }
playlist-track-source-on-usb = Sur la clé USB : { $path }
playlist-track-source-local = Dans la bibliothèque locale : { $path }

playlist-membership-all = Déjà dedans
playlist-membership-some = { $contained } sur { $selected } dedans

playlist-reorder-lock-tooltip = Pas de réordonnancement sur la clé USB — « { $name } » y existe déjà, et l'export additif garde l'ordre de ses morceaux tel quel. Les nouveaux morceaux sont quand même ajoutés dans l'ordre choisi.

playlist-error-name-empty = le nom de la playlist ne doit pas être vide
playlist-create-failed = Impossible de créer la playlist : { $error }
playlist-folder-error-name-empty = le nom du dossier ne doit pas être vide
playlist-folder-error-move-into-playlist = seul un dossier peut contenir d'autres éléments
playlist-folder-error-move-into-itself = un dossier ne peut pas aller dans lui-même
playlist-folder-create-failed = Impossible de créer le dossier : { $error }
playlist-rename-failed = Échec du renommage : { $error }
playlist-order-failed = Échec de l'enregistrement de l'ordre des playlists : { $error }
playlist-track-order-failed = Échec de l'enregistrement de l'ordre des morceaux : { $error }
playlist-delete-failed = Échec de la suppression : { $name }

## playlist-import

playlist-import-note-updates = { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} · met à jour « { $name } »

playlist-import-one-imported = Playlist importée { $name } : { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}
playlist-import-one-updated = Playlist mise à jour { $name } : { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}
playlist-import-many = { $imported } importée(s) sur { $count ->
    [one] { $count } playlist
   *[other] { $count } playlists
} : { $tracks ->
    [one] { $tracks } morceau
   *[other] { $tracks } morceaux
}
playlist-import-skipped = { $count ->
    [one] { $count } morceau ignoré
   *[other] { $count } morceaux ignorés
} (voir le journal des événements)
playlist-import-failed-lists = { $count } en échec : { $names } (voir le journal des événements)

playlist-import-error-nothing-imported = { $count ->
    [one] le seul morceau de « { $playlist } » n'a pas pu être importé
   *[other] aucun des { $count } morceaux de « { $playlist } » n'a pu être importé
} : fichiers introuvables sur cet ordinateur ou non pris en charge
playlist-import-list-failed = Impossible de lire les playlists { $library } : { $error }

playlist-import-item-failed = Importation de playlist { $library } : { $playlist } : { $error }
playlist-import-one-failed = Échec de l'importation de playlist { $library } : { $error }
playlist-import-many-failed = Échec de l'importation des playlists : { $error }

## selection-bar

selection-bar-duration-unknown = { $count ->
    [one] { $count } des morceaux sélectionnés n'a pas encore de durée
   *[other] { $count } des morceaux sélectionnés n'ont pas encore de durée
}
selection-bar-duration = · { $duration }
selection-bar-duration-at-least = · { $duration }+

## group-edit

group-edit-save-failed = Impossible d'enregistrer les détails des morceaux : { $error }

## track-detail

track-detail-saved = Détails enregistrés pour { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}
track-detail-saved-usb = Détails enregistrés sur la clé USB pour { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
}


track-detail-progress-checking-tracks = USB : vérification des morceaux
track-detail-progress-writing-pdb = USB : écriture de la PDB
track-detail-progress-writing-edb = USB : écriture de l'eDB

track-detail-error-release-date = la date de sortie doit être au format AAAA, AAAA-MM ou AAAA-MM-JJ, de { $min } à { $max }

## track-detail-cues

track-detail-cues-saved = { $count ->
    [one] { $count } cue enregistré
   *[other] { $count } cues enregistrés
}
track-detail-cues-saved-no-cache = { $count ->
    [one] { $count } cue enregistré
   *[other] { $count } cues enregistrés
} (cache d'analyse pas encore mis à jour)
track-detail-cues-saved-usb = { $count ->
    [one] { $count } cue enregistré
   *[other] { $count } cues enregistrés
} sur la clé USB


track-detail-cues-progress-checking-track = USB : vérification du morceau
track-detail-cues-progress-writing-analysis-files = USB : écriture des fichiers d'analyse
track-detail-cues-progress-updating-library-track = Mise à jour du morceau dans la bibliothèque

track-detail-cues-error-not-in-usb-database = ce morceau n'est pas dans la base de données des lecteurs de la clé USB, il ne peut donc pas être modifié sur la clé ({ $path }). « Aperçu des corrections » montre ce qui peut être réparé.
usb-warning-track-missing-from-database = { $path } n'est pas dans la { $database } de la clé USB, la modification n'a donc été enregistrée que dans l'autre base de données. « Aperçu des corrections » montre la réparation qui l'ajoute.
track-detail-cues-error-unknown-key = tonalité inconnue : { $key }
track-detail-cues-error-too-many = { $max } points de cue au maximum sont autorisés
track-detail-cues-error-bpm-range = le BPM doit être supérieur à 0 et au plus 999
track-detail-save-cues-failed = Impossible d'enregistrer les cues : { $error }

## playback

playback-source-library = Bibliothèque
playback-source-usb = USB
playback-playing = Lecture depuis { $source } : { $track }

## track-table

track-table-source-user = Défini manuellement
track-table-source-mixxx = Depuis Mixxx
track-table-source-rekordbox = Depuis rekordbox
track-table-source-analyzer = Analysé avec : { $analyzer }

track-table-format-unknown = Inconnu
track-table-format-sample-rate = { NUMBER($khz) } kHz
track-table-format-bit-depth = { $bits } bits
track-table-format-bitrate = { $kbps } kbps

## track-row

track-row-unknown-title = Titre inconnu
track-row-unknown-artist = Artiste inconnu

## ui-add-to

ui-add-to-apply = Ajouter
ui-add-to-create = Créer
ui-add-to-list-aria-label = Playlists
ui-add-to-new-name-aria-label = Nom de la nouvelle playlist
ui-add-to-new-name-placeholder = Nouvelle playlist
ui-add-to-title = Ajouter aux playlists
ui-add-to-title-count = { $count ->
    [one] Ajouter { $count } morceau aux playlists
   *[other] Ajouter { $count } morceaux aux playlists
}
ui-add-to-empty = Aucune playlist pour l'instant : créez-en une ci-dessous.

## ui-backups

ui-backups-delete-title = Supprimer la sauvegarde
ui-backups-intro = Chaque sauvegarde contient les deux bases de données de la clé USB : l'ancienne (PDB) et la nouvelle (eDB).
ui-backups-load-failed = Impossible de charger les sauvegardes
ui-backups-refresh = Actualiser
ui-backups-restore = Restaurer
ui-backups-restore-title = Restaurer la sauvegarde

## ui-common

ui-common-remove = Retirer
ui-common-delete = Supprimer
ui-common-dismiss = Fermer
ui-common-drag-to-reorder = Glisser pour réordonner
ui-common-play = Lecture
ui-common-play-from-here-tooltip = Lire à partir d'ici
ui-common-reanalyze = Réanalyser
ui-common-remove-from-playlist = Retirer de la playlist
ui-common-analyze-failed = Échec de l'analyse : { $error }
ui-common-playback-failed = Échec de la lecture : { $error }
ui-common-no-entries = Aucune entrée
ui-common-import = Importer
ui-common-idle = En attente
ui-common-update-download = Télécharger
ui-common-update-restart = Mettre à jour et redémarrer
ui-common-genre = Genre
ui-common-release-date = Date de sortie
ui-common-rating = Note
ui-common-backups = Sauvegardes
ui-common-event-log = Journal des événements
ui-common-settings = Paramètres
ui-common-media-library = Médiathèque
ui-common-usb-playlists = Playlists USB
ui-common-usb = USB
ui-common-language = Langue
ui-common-loading-tracks = Chargement des morceaux...
ui-common-loading-playlists = Chargement des playlists...
ui-common-cancel = Annuler
ui-common-confirm = Confirmer
ui-common-save = Enregistrer

## ui-convert

ui-convert-folder-hint = Enregistrés dans { $folder } (modifiable dans les Paramètres).
ui-convert-apply = Convertir
ui-convert-format = Format
ui-convert-intro = Crée un autre fichier de chaque morceau sans perte, OGG ou Opus sélectionné, pour les lecteurs qui ne lisent pas le FLAC, l'ALAC, l'OGG ou l'Opus. Cela reste le même morceau : aucune nouvelle ligne dans la bibliothèque, et les cues et modifications restent sur le morceau.
ui-convert-remove = Supprimer les transcodages
ui-convert-remove-tooltip = Supprimer les fichiers transcodés des morceaux sélectionnés
ui-convert-title = Convertir des morceaux
ui-convert-title-count = { $count ->
    [one] Convertir { $count } morceau
   *[other] Convertir { $count } morceaux
}

## ui-cover-img

ui-cover-img-alt = pochette

## ui-drive-name

ui-drive-name-conflict = Cette clé porte le nom « { $name } », mais ce n'est pas la clé que cet ordinateur connaît sous ce nom : c'en est très probablement une copie. Donnez-lui un nom à elle pour que les deux ne partagent pas leurs sauvegardes.
ui-drive-name-input-placeholder = ex. Clé Club
ui-drive-name-intro = Donnez un nom à cette clé USB pour que les sauvegardes et le cache local la suivent de façon fiable, même sur un autre ordinateur. Si c'est une vraie clé USB avec un nom de volume, il est déjà rempli ci-dessous : modifiez-le ou remplacez-le par le vôtre. Chaque clé a besoin de son propre nom, même quand plusieurs ont le même nom de volume.
ui-drive-name-rename-note = Les sauvegardes et le cache local de la clé la suivent sous son nouveau nom.
ui-drive-name-skip = Plus tard
ui-drive-name-title-name = Nommer cette clé
ui-drive-name-title-rename = Renommer cette clé

## ui-event-log

ui-event-log-clear = Effacer
ui-event-log-coalesced-occurrences-tooltip = Occurrences regroupées
ui-event-log-level = Niveau
ui-event-log-level-filter-all = Tous
ui-event-log-level-filter-error = Erreur
ui-event-log-level-filter-info = Info
ui-event-log-level-filter-warn = Avertissement
ui-event-log-row-level-error = erreur
ui-event-log-row-level-info = info
ui-event-log-row-level-warn = avert.
ui-event-log-open-file = Ouvrir le fichier journal
ui-event-log-open-file-tooltip = Ouvrir le journal complet de cette session dans un éditeur de texte -- la liste ici ne garde que les entrées les plus récentes
ui-event-log-source = Source
ui-event-log-button-new-errors = { $count ->
    [one] Journal des événements, { $count } nouvelle erreur
   *[other] Journal des événements, { $count } nouvelles erreurs
}
ui-event-log-button-linked-details = Journal des événements, contient des détails sur le rapport USB
ui-event-log-summary-count = { $count ->
    [one] { $count } événement
   *[other] { $count } événements
}
ui-event-log-summary-count-repeats = { $count ->
    [one] { $count } événement ({ $occurrences } occurrences)
   *[other] { $count } événements ({ $occurrences } occurrences)
}
ui-event-log-unknown-code = inconnu
ui-event-log-empty = Aucun événement
ui-event-log-open-file-failed = Impossible d'ouvrir le fichier journal : { $error }

## ui-export-playlists

ui-export-playlists-title = Exporter des playlists vers la clé USB
ui-export-playlists-title-usb = Exporter des playlists vers { $name }
ui-export-playlists-select-all = Sélectionner toutes les playlists
ui-export-playlists-select-changed = Nouvelles et modifiées
ui-export-playlists-select-changed-tooltip = Sélectionner seulement les playlists que la clé USB n'a pas encore ou qui ont changé depuis leur dernier export
ui-export-playlists-table-aria-label = Playlists à exporter
ui-export-playlists-col-tracks = Morceaux
ui-export-playlists-col-size = Taille
ui-export-playlists-col-copy = À copier
ui-export-playlists-col-status = État sur la clé
ui-export-playlists-col-last-exported = Dernier export
ui-export-playlists-empty = Aucune playlist à exporter. Branchez une clé USB et créez d'abord une playlist.
ui-export-playlists-count = { $count } sur { $total } sélectionnées
ui-export-playlists-force-order = Utiliser l'ordre des playlists de l'application
ui-export-playlists-force-order-needs-all = Sélectionnez toutes les playlists pour l'utiliser
ui-export-playlists-force-order-hint = Les playlists vont en tête de la liste de la clé USB, dans cet ordre. Sans cette option, les playlists déjà sur la clé gardent leur place et les nouvelles vont en tête.
ui-export-playlists-force-track-order = Utiliser l'ordre des morceaux de l'application dans les playlists déjà sur la clé
ui-export-playlists-all-warning = Exporter toutes les playlists peut prendre longtemps.
ui-export-playlists-run = { $count ->
    [one] Exporter { $count } playlist
   *[other] Exporter { $count } playlists
}
ui-export-playlists-run-usb = { $count ->
    [one] Exporter { $count } playlist vers { $name }
   *[other] Exporter { $count } playlists vers { $name }
}
ui-export-playlists-load-failed = Impossible de charger les playlists à exporter : { $error }

## ui-group-edit

ui-group-edit-apply = Appliquer
ui-group-edit-intro = Seuls les champs que vous modifiez sont écrits ; les autres restent tels quels sur chaque morceau. Un champ vide efface la valeur.
ui-group-edit-mixed = Variable
ui-group-edit-release-date-tooltip = AAAA-MM-JJ, ou simplement AAAA-MM ou AAAA : l'export complète le mois ou l'année
ui-group-edit-release-date-placeholder = AAAA-MM-JJ
ui-group-edit-title = Modifier des morceaux
ui-group-edit-title-count = { $count ->
    [one] Modifier { $count } morceau
   *[other] Modifier { $count } morceaux
}
ui-group-edit-waiting-for-usb-job = Attente de la fin de la tâche USB en cours avant d'enregistrer...

## ui-header

ui-header-usb-name-badge-unnamed = Sans nom
ui-header-usb-name-badge-not-connected = Non branchée
ui-header-usb-name-badge-name-tooltip = Nommer cette clé
ui-header-usb-name-badge-rename-tooltip = Renommer cette clé

ui-header-badge-none = Aucune playlist active
ui-header-help = Aide
ui-header-playlist-badge = Playlist
ui-header-settings-update-available = Paramètres (mise à jour disponible)
ui-header-update-banner-view-release = voir la version

## ui-help

ui-help-aria-label = Aide de démarrage rapide
ui-help-close-aria-label = Fermer l'aide
ui-help-donate-heading = Soutenir DJ USB Tkit
ui-help-donate-open = Ouvrir la page de soutien
ui-help-donate-text = Les options de soutien sont tenues à jour sur chiph.art, pour que les liens de paiement restent valides.
ui-help-step-analyze = Analyser :
ui-help-step-analyze-text = Cliquez sur « Analyser la bibliothèque » pour repérer les changements dans vos dossiers et analyser leurs morceaux.
ui-help-step-export = Exporter :
ui-help-step-export-text = Ouvrez votre playlist et cliquez sur « Exporter vers la clé USB » pour l'écrire sur votre clé.
ui-help-step-folders = Ajouter des dossiers :
ui-help-step-folders-text = Cliquez sur « + Ajouter un dossier » dans la Bibliothèque pour ajouter vos dossiers de musique.
ui-help-step-playlist = Créer une playlist :
ui-help-step-playlist-text = Cliquez sur « + Nouvelle » dans la barre latérale pour créer une playlist, puis ajoutez-y des morceaux depuis la Bibliothèque ou la clé USB.
ui-help-step-usb = Brancher la clé USB :
ui-help-step-usb-text = Allez dans USB et choisissez votre clé USB External library.
ui-help-title = Démarrage rapide

## ui-job

ui-job-cancel-analysis = Annuler l'analyse
ui-job-cancel-transcode-aria-label = Annuler la conversion
ui-job-cancel-transcode-title = Annuler la conversion (les fichiers terminés sont conservés)
ui-job-working = En cours...
ui-job-paused = en pause
ui-job-done = Terminé
ui-job-failed = Échec
ui-job-resume-analysis = Reprendre l'analyse
ui-job-pause-analysis = Mettre l'analyse en pause
ui-job-progress = Progression
ui-job-progress-aria-label = Progression de l'opération

## ui-library

ui-library-add-source = + Ajouter un dossier
ui-library-analyze-library = Analyser la bibliothèque
ui-library-external-rekordbox-db = Bibliothèque externe
ui-library-filter-source-aria-label = Filtrer par source
ui-library-folders = Dossiers
ui-library-import-from-mixxx = Importer depuis Mixxx
ui-library-import-from-rekordbox = Importer depuis rekordbox
ui-library-libraries = Bibliothèques
ui-library-search-placeholder = Rechercher titre, artiste, album, genre, année
ui-library-source-filter-indicator = Filtré
ui-library-sources = Sources
ui-library-toggle-mixxx-library-aria-label = Activer/désactiver la bibliothèque Mixxx
ui-library-toggle-rekordbox-library-aria-label = Activer/désactiver la bibliothèque rekordbox
ui-library-chip-not-imported-rekordbox-tooltip = Pas encore importée : cliquez sur ↻ pour importer depuis rekordbox
ui-library-chip-not-imported-mixxx-tooltip = Pas encore importée : cliquez sur ↻ pour importer depuis Mixxx
ui-library-folder-missing-tooltip = Dossier source introuvable. Cliquez pour le relocaliser.
ui-library-folder-missing-aria-label = Dossier source introuvable
ui-library-folder-missing-path-tooltip = Dossier introuvable. Cliquez pour le relocaliser : { $path }
ui-library-relocate-starting = Relocaliser le dossier source : { $path }
ui-library-empty-heading = Ajoutez un dossier de musique pour commencer
ui-library-empty-add-folder = Ajouter un dossier
ui-library-empty-import-rekordbox = Bibliothèque rekordbox
ui-library-empty-import-mixxx = Bibliothèque Mixxx
ui-library-add-folders-progress = Chargement de la médiathèque...
ui-library-add-folders-progress-done = Chargement de la médiathèque terminé
ui-library-add-folders-progress-failed = Échec du chargement de la médiathèque
ui-library-add-folders-indexing = Indexation des nouveaux fichiers...
ui-library-add-folders-building = Construction de la liste des morceaux...
ui-library-add-folders-refreshing = Actualisation des playlists...
ui-library-remove-folder-title = Retirer le dossier source
ui-library-remove-folder-confirm = Retirer le dossier source ?

    { $path }

    Tous les morceaux de ce dossier seront retirés de la bibliothèque. Les morceaux qui sont dans une playlist locale seront aussi retirés de ces playlists. Les morceaux qui se trouvent aussi dans un autre dossier source restent dans la bibliothèque.
ui-library-import-rekordbox-starting = Importation depuis la bibliothèque de bureau...
ui-library-import-rekordbox-failed = Échec de l'importation de la bibliothèque de bureau : { $error }
ui-library-import-mixxx-starting = Importation depuis la bibliothèque Mixxx...
ui-library-import-mixxx-failed = Échec de l'importation de la bibliothèque Mixxx : { $error }

## ui-playlist

ui-playlist-analyze-missing = Analyser les morceaux non analysés
ui-playlist-delete-playlist = Supprimer la playlist
ui-playlist-export = Exporter vers la clé USB
ui-playlist-export-space-tooltip = La taille à copier comprend les fichiers d'analyse et les pochettes des morceaux, elle peut donc dépasser la taille audio indiquée dans le titre
ui-playlist-export-status = Pas encore exportée.
ui-playlist-format-warning-tooltip = Suit les formats audio de la clé USB (vue USB). Seuls les morceaux sans perte, OGG et Opus peuvent être convertis
ui-playlist-name-placeholder = Nom de la playlist...
ui-playlist-rename-aria-label = Renommer la playlist
ui-playlist-rename-tooltip = Renommer la playlist (ou double-cliquer sur le nom)
ui-playlist-search-placeholder = Rechercher dans les morceaux de la playlist
ui-playlist-title = Playlist
ui-playlist-active-tooltip = Playlist active
ui-playlist-none-active = Créez et activez d'abord une playlist
ui-playlist-imported-from = · Importée depuis { $library }
ui-playlist-export-failed = Échec de l'export : { $error }
ui-playlist-export-failed-unknown = erreur inconnue
ui-playlist-track-order-still-loading = Ordre des morceaux non enregistré : la liste était encore en chargement. Réessayez.
ui-playlist-name-required = Le nom de la playlist est obligatoire
ui-playlist-create-progress = Création de la playlist...
ui-playlist-create-progress-done = Création de la playlist terminée
ui-playlist-create-progress-failed = Échec de la création de la playlist
ui-playlist-create-saving = Enregistrement de la playlist...
ui-playlist-delete-title = Supprimer la playlist de l'application
ui-playlist-add-progress-done = Ajout des morceaux terminé
ui-playlist-add-progress-failed = Échec de l'ajout des morceaux
ui-playlist-add-resolving = Recherche des morceaux...
ui-playlist-add-adding = Ajout des morceaux...
ui-playlist-add-refreshing = Actualisation de la playlist...
ui-playlist-add-none-resolved = Aucun identifiant de morceau importé trouvé à ajouter
ui-playlist-add-select-first = Sélectionnez au moins un morceau à ajouter
ui-playlist-empty-heading = Parcourez la Bibliothèque ou la clé USB pour ajouter des morceaux

## import-conflicts

import-conflicts-title = Données de morceaux différentes dans { $library }
import-conflicts-message = { $library } a des cues, un BPM, une tonalité ou une grille de temps différents pour { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} par rapport à votre bibliothèque.
import-conflicts-use = Utiliser { $library }
import-conflicts-field-cues = cues
import-conflicts-field-bpm = BPM
import-conflicts-field-key = tonalité
import-conflicts-field-grid = grille de temps
import-conflicts-applied = Cues, BPM, tonalité et grille de temps de { $count ->
    [one] { $count } morceau
   *[other] { $count } morceaux
} repris de { $library }
ui-import-conflicts-keep = Garder la bibliothèque
ui-import-conflicts-list-aria-label = Morceaux différents

## ui-playlist-import

ui-playlist-import-intro = Chacune est importée comme nouvelle playlist. Les morceaux qui ne sont pas encore dans la bibliothèque sont importés avec elle.
ui-playlist-import-ok-count = { $count ->
    [one] Importer { $count }
   *[other] Importer { $count }
}
ui-playlist-import-column-aria-label = Listes { $library }
ui-playlist-import-title = Importer des playlists
ui-playlist-import-group-playlists = Playlists
ui-playlist-import-group-crates = Bacs
ui-playlist-import-group-history = Historique
ui-playlist-import-group-select-all = Tout sélectionner : { $group }
ui-playlist-import-none-found = Aucune playlist avec des morceaux trouvée
ui-playlist-import-one-progress = Importation de la playlist { $library }...
ui-playlist-import-one-progress-done = Importation de la playlist { $library } terminée
ui-playlist-import-one-progress-failed = Échec de l'importation de la playlist { $library }
ui-playlist-import-one-starting = Importation de { $name }...
ui-playlist-import-many-progress = { $count ->
    [one] Importation de { $count } playlist...
   *[other] Importation de { $count } playlists...
}
ui-playlist-import-many-progress-done = { $count ->
    [one] Importation de { $count } playlist terminée
   *[other] Importation de { $count } playlists terminée
}
ui-playlist-import-many-progress-failed = { $count ->
    [one] Échec de l'importation de { $count } playlist
   *[other] Échec de l'importation de { $count } playlists
}
ui-playlist-import-many-starting = Importation des playlists...

## ui-playback

ui-playback-paused = En pause
ui-playback-not-found = Lecture impossible : morceau introuvable dans la Bibliothèque ou sur la clé USB sélectionnée.
ui-playback-failed-no-message = Échec de la lecture

## ui-startup

ui-startup-init-failed = Échec de l'initialisation : { $error }
ui-startup-progress = Initialisation...
ui-startup-progress-done = Initialisation terminée
ui-startup-progress-failed = Échec de l'initialisation

## ui-update

ui-update-install-failed = Impossible d'installer la version { $version }.

## ui-rating

ui-rating-1-star-aria-label = 1 étoile
ui-rating-2-stars-aria-label = 2 étoiles
ui-rating-3-stars-aria-label = 3 étoiles
ui-rating-4-stars-aria-label = 4 étoiles
ui-rating-5-stars-aria-label = 5 étoiles
ui-rating-tooltip = Cliquez sur une étoile pour noter ; cliquez à nouveau sur la même étoile pour effacer

## ui-selection-bar

ui-selection-bar-count = { $count ->
    [one] { $count } sélectionné
   *[other] { $count } sélectionnés
}

ui-selection-bar-add-to = Ajouter à…
ui-selection-bar-add-to-target = Ajouter à
ui-selection-bar-add-to-tooltip = Choisir une ou plusieurs playlists où ajouter les morceaux sélectionnés
ui-selection-bar-add-tooltip = Ajouter les morceaux sélectionnés à { $playlist }
ui-selection-bar-select-all-progress = Sélection de tous les morceaux correspondants...
ui-selection-bar-select-all-progress-done = Sélection de tous les morceaux correspondants terminée
ui-selection-bar-select-all-progress-failed = Échec de la sélection de tous les morceaux correspondants
ui-selection-bar-enumerating = Énumération des morceaux...
ui-selection-bar-clear-selection = Effacer la sélection
ui-selection-bar-convert = Convertir…
ui-selection-bar-convert-tooltip = Créer des fichiers AIFF, WAV ou MP3 des morceaux sans perte, OGG et Opus sélectionnés pour les lecteurs qui ne lisent pas le FLAC, l'ALAC, l'OGG ou l'Opus, ou les supprimer
ui-selection-bar-count-none = 0 sélectionné
ui-selection-bar-edit-details = Modifier les détails…
ui-selection-bar-edit-short = Modifier…
ui-selection-bar-edit-tooltip = Modifier le genre, la date de sortie et la note des morceaux sélectionnés
ui-selection-bar-hint-library = Sélectionnez des morceaux pour les ajouter, les réanalyser, les convertir ou les modifier
ui-selection-bar-hint-playlist = Sélectionnez des morceaux pour les ajouter, les retirer, les réanalyser, les convertir ou les modifier
ui-selection-bar-hint-usb = Sélectionnez des morceaux pour les ajouter à une playlist ou les modifier
ui-selection-bar-reanalyze-tooltip = Remettre la forme d'onde, le BPM, la tonalité et le premier temps des morceaux sélectionnés à une nouvelle analyse, en remplaçant les valeurs importées ou modifiées (les cues sont conservés)
ui-selection-bar-remove-tooltip = Retirer les morceaux sélectionnés de cette playlist (ils restent dans la bibliothèque)

## ui-settings

ui-settings-accent-reset = Réinitialiser
ui-settings-analysis = Analyse
ui-settings-analysis-engine-stratum = Stratum (intégré)
ui-settings-backup-before-export = Sauvegarder la PDB et l'eDB avant l'export
ui-settings-backup-before-export-hint = Avant chaque export, copie la PDB et l'eDB dans un dossier de sauvegardes horodaté, à côté des originaux.
ui-settings-backups-to-keep = Sauvegardes à conserver par fichier
ui-settings-backups-to-keep-hint = La sauvegarde la plus récente reste toujours sur la clé USB ; les plus anciennes sont archivées sur cet ordinateur jusqu'à ce nombre, puis supprimées.
ui-settings-bpm-engine = Moteur BPM
ui-settings-bpm-range = Plage de BPM
ui-settings-close-aria-label = Fermer les paramètres
ui-settings-engine-library = Écrire la bibliothèque Engine DJ
ui-settings-engine-library-hint = Chaque fois que l'application modifie une clé USB, reconstruit la bibliothèque que lisent les lecteurs Denon Prime et Numark Mixstream, pour qu'ils utilisent ses beatgrids, ses cues et ses playlists sans nouvelle importation ni nouvelle analyse. L'historique de lecture est conservé ; les modifications faites sur le lecteur lui-même (cues, boucles, playlists) sont remplacées.
ui-settings-essentia-download = Télécharger ~5 Mo
ui-settings-essentia-ready = ✓ Essentia prêt
ui-settings-essentia-downloading = Téléchargement...
ui-settings-essentia-not-installed = Fichiers Essentia non installés
ui-settings-essentia-download-failed = Échec du téléchargement : { $error }
ui-settings-essentia-downloading-progress = Téléchargement d'Essentia...
ui-settings-essentia-installed = Essentia installé
ui-settings-essentia-removed = Essentia supprimé
ui-settings-export-backup-enabled = Sauvegarde à l'export : activée
ui-settings-export-backup-disabled = Sauvegarde à l'export : désactivée
ui-settings-engine-library-enabled = Bibliothèque Engine DJ à l'export : activée
ui-settings-engine-library-disabled = Bibliothèque Engine DJ à l'export : désactivée
ui-settings-backup-retention-saved = Sauvegardes à conserver par fichier : { $value }
ui-settings-bpm-range-saved = Plage de BPM de l'analyse : { $range }
ui-settings-key-notation-saved = Notation des tonalités : { $notation }
ui-settings-key-notation-failed = Impossible d'enregistrer la notation des tonalités
ui-settings-bpm-engine-saved = Moteur BPM : { $engine }
ui-settings-bpm-engine-changed = Moteur BPM changé pour { $engine }
ui-settings-key-engine-saved = Moteur de tonalité : { $engine }
ui-settings-key-engine-changed = Moteur de tonalité changé pour { $engine }
ui-settings-version = Version { $version }
ui-settings-version-not-set = Non définie
ui-settings-export = Export
ui-settings-get-node-js = Obtenir Node.js
ui-settings-key-engine = Moteur de tonalité
ui-settings-key-notation = Notation des tonalités
ui-settings-key-notation-camelot = Camelot (8A)
ui-settings-key-notation-classic = Classique (Am)
ui-settings-node-js-missing = Node.js introuvable —
ui-settings-sync-mode = Mode de synchronisation
ui-settings-sync-mode-additive = Mise à jour additive des playlists
ui-settings-sync-mode-additive-hint = Ajoute les morceaux manquants aux playlists de même nom sur la clé USB, mais garde les morceaux déjà présents.
ui-settings-sync-mode-mirror = Playlist miroir (correspondance exacte)
ui-settings-sync-mode-mirror-hint = Les playlists de même nom sur la clé USB sont réécrites pour correspondre exactement à cette playlist de l'application (les anciens morceaux sont retirés).
ui-settings-theme = Thème
ui-settings-theme-auto = Auto
ui-settings-theme-dark = Sombre
ui-settings-theme-light = Clair
ui-settings-transcode-folder-change = Changer…
ui-settings-transcode-folder-default-tag = Dossier par défaut
ui-settings-transcode-folder-hint = Où Convertir… enregistre les fichiers AIFF, WAV et MP3 qu'il crée. Gardez-le en dehors de vos dossiers de musique, pour que les autres applications de musique n'affichent pas les fichiers en double. Un nouveau dossier ne s'applique qu'aux nouveaux fichiers.
ui-settings-transcode-folder-reset = Utiliser le dossier par défaut
ui-settings-transcodes = Transcodages
ui-settings-update-check = Rechercher des mises à jour
ui-settings-update-release-link = Nouveautés ↗

## ui-sidebar

ui-sidebar-collapse = Réduire la barre latérale
ui-sidebar-donate = Faire un don
ui-sidebar-donate-tooltip = Pensez à faire un don
ui-sidebar-export-playlists = Exporter des playlists…
ui-sidebar-export-playlists-tooltip = Exporter plusieurs playlists vers la clé USB en une fois
ui-sidebar-expand = Déplier la barre latérale
ui-sidebar-folder-delete = Supprimer le dossier
ui-sidebar-folder-delete-title = Supprimer le dossier
ui-sidebar-folder-name-placeholder = Nom du dossier
ui-sidebar-folder-new-playlist = Nouvelle playlist dans ce dossier
ui-sidebar-folder-rename = Renommer le dossier
ui-sidebar-history = Historique
ui-sidebar-import-playlist-aria-label = Importer des playlists
ui-sidebar-import-playlist-tooltip = Importer des playlists depuis rekordbox ou Mixxx
ui-sidebar-library = Bibliothèque
ui-sidebar-manage = Gérer
ui-sidebar-nav-aria-label = Sections de l'application
ui-sidebar-new-folder = Nouveau dossier, dans le dossier sélectionné
ui-sidebar-new-playlist = Nouvelle playlist, dans le dossier sélectionné
ui-sidebar-player-menu = Menu du lecteur
ui-sidebar-playlists = Playlists
ui-sidebar-resizer-tooltip = Glisser pour redimensionner · double-cliquer pour réinitialiser
ui-sidebar-website = Site web

## ui-track-detail

ui-track-detail-add-cue = + Hot cue
ui-track-detail-add-memory-cue-aria-label = Ajouter un memory cue
ui-track-detail-add-memory-cue-tooltip = Ajouter un memory cue à la tête de lecture (M)
ui-track-detail-beat-grid = Beatgrid
ui-track-detail-bpm = BPM
ui-track-detail-bpm-double-tooltip = Doubler le BPM (l'analyse a détecté la moitié du tempo)
ui-track-detail-bpm-half-tooltip = Diviser le BPM par deux (l'analyse a détecté le double du tempo)
ui-track-detail-bpm-minus-tooltip = -0,01 BPM
ui-track-detail-bpm-plus-tooltip = +0,01 BPM
ui-track-detail-close-aria-label = Fermer
ui-track-detail-cues = Cues
ui-track-detail-delete-cue-aria-label = Supprimer le cue
ui-track-detail-delete-memory-cue-aria-label = Supprimer le memory cue
ui-track-detail-first-beat = Premier temps
ui-track-detail-first-beat-minus-tooltip = Reculer d'un temps
ui-track-detail-first-beat-plus-tooltip = Avancer d'un temps
ui-track-detail-follow-grid-aria-label = Les cues suivent la grille
ui-track-detail-follow-grid-tooltip = Les cues suivent la grille : changer le BPM ou le premier temps déplace chaque cue pour qu'il reste sur son temps. Désactivé, les cues gardent leur place dans l'audio, comme dans rekordbox
ui-track-detail-genre-tooltip = Genre : saisissez-en un ou choisissez-en un déjà présent dans la bibliothèque
ui-track-detail-grid-level-tooltip = Intensité d'affichage de la beatgrid sur la forme d'onde
ui-track-detail-hint-aria-label = Comment utiliser la forme d'onde
ui-track-detail-hint-cue-markers = Marqueurs de cue
ui-track-detail-hint-cue-markers-drag = glisser pour déplacer ·
ui-track-detail-hint-cue-markers-free = place librement · cliquer pour lire depuis le cue ·
ui-track-detail-hint-cue-markers-memory = ajoute un memory cue (▼)
ui-track-detail-hint-cue-markers-quantize = activé : aimante aux temps,
ui-track-detail-hint-shift-key = Maj
ui-track-detail-hint-tooltip = Forme d'onde : cliquer pour déplacer la tête de lecture, glisser la tête de lecture pour écouter, double-cliquer pour ajouter un cue, faire défiler pour zoomer, glisser pour se déplacer. Marqueurs de cue : glisser pour déplacer, cliquer pour lire depuis le cue. Avec Q activé, les cues s'aimantent aux temps ; maintenir Maj pour placer librement. Touches : Espace lecture/pause · C ajouter un cue · M ajouter un memory cue · 1–8 aller au cue · ←/→ déplacer le cue sélectionné d'un temps (Maj : 10 ms) · Ctrl+Z annuler · Ctrl+Maj+Z rétablir.
ui-track-detail-hint-waveform = Forme d'onde
ui-track-detail-hint-waveform-text = cliquer pour déplacer la tête de lecture · glisser la tête de lecture pour écouter · double-cliquer pour ajouter un cue · faire défiler pour zoomer · glisser pour se déplacer
ui-track-detail-key = Tonalité
ui-track-detail-key-minus-tooltip = Tonalité précédente
ui-track-detail-key-plus-tooltip = Tonalité suivante
ui-track-detail-memory-cue-badge = memory cue
ui-track-detail-memory-cue-colour-aria-label = Couleur du memory cue
ui-track-detail-memory-cue-only-tooltip = Memory cue seulement (pas de hot cue)
ui-track-detail-memory-cues = Memory cues :
ui-track-detail-metronome-aria-label = Métronome
ui-track-detail-metronome-mix = Mix
ui-track-detail-metronome-mix-tooltip = Mix du métronome : à gauche la musique domine, à droite les clics ; au milieu les deux jouent à plein niveau
ui-track-detail-metronome-tooltip = Métronome : un clic sur chaque temps pendant la lecture, pour entendre si la grille tombe juste
ui-track-detail-ms = ms
ui-track-detail-name-placeholder = Nom
ui-track-detail-next-memory-cue = Memory cue suivant
ui-track-detail-no-colour = Sans couleur
ui-track-detail-overview-aria-label = Morceau entier : cliquer ou glisser pour déplacer la vue
ui-track-detail-play-from-cue-aria-label = Lire depuis ce cue
ui-track-detail-play-from-memory-cue-aria-label = Lire depuis ce memory cue
ui-track-detail-playback-start = Début de lecture
ui-track-detail-previous-memory-cue = Memory cue précédent
ui-track-detail-quantize = Q
ui-track-detail-quantize-tooltip = Quantize : les cues nouveaux et déplacés s'aimantent au temps le plus proche (maintenir Maj pour placer librement)
ui-track-detail-redo = Rétablir
ui-track-detail-redo-tooltip = Rétablir (Ctrl+Maj+Z)
ui-track-detail-release-date-tooltip = Date de sortie : AAAA-MM-JJ, ou simplement AAAA-MM ou AAAA (l'export complète le mois ou l'année) ; vide pour aucune
ui-track-detail-reveal-aria-label = Afficher dans le dossier
ui-track-detail-start = La lecture commence à
ui-track-detail-start-first-cue = Premier cue
ui-track-detail-start-first-cue-tooltip = Le CDJ charge le morceau sur son premier cue : le cue A, ou un memory cue placé avant
ui-track-detail-start-note = Pas encore de points de cue : le CDJ démarre au début de l'audio
ui-track-detail-title = Points de cue
ui-track-detail-undo = Annuler
ui-track-detail-undo-tooltip = Annuler (Ctrl+Z)
ui-track-detail-zoom-fit = Ajuster
ui-track-detail-zoom-fit-tooltip = Afficher le morceau entier
ui-track-detail-zoom-in = Zoom avant
ui-track-detail-zoom-out = Zoom arrière
ui-track-detail-pause = Pause
ui-track-detail-marker-playback-start-tooltip = Début de lecture · { $time }
ui-track-detail-marker-memory-cue-tooltip = Memory cue · { $time }
ui-track-detail-marker-memory-cue-named-tooltip = Memory cue · { $name } · { $time }
ui-track-detail-marker-hot-cue-tooltip = { $cue } · { $time }
ui-track-detail-cue-colour-aria-label = Couleur du cue { $cue }
ui-track-detail-start-first-beat-tooltip = Ajoute un memory cue ▶ (sans hot cue) sur le premier temps, pour que le CDJ charge le morceau là au lieu du cue A
ui-track-detail-start-marker = Marqueur de début
ui-track-detail-start-marker-tooltip = Le CDJ charge le morceau sur le marqueur de début ▶ que vous avez placé ; glissez-le sur la forme d'onde pour le déplacer
ui-track-detail-whole-track = Morceau entier
ui-track-detail-reveal-tooltip = Afficher dans le dossier : { $path }
ui-track-detail-reveal-failed = Impossible d'afficher le fichier : { $error }
ui-track-detail-connect-usb-first = Branchez la clé USB où se trouve ce morceau avant de modifier ses cues.
ui-track-detail-analyze-first = Analysez d'abord ce morceau pour modifier ses cues.
ui-track-detail-open-failed = Impossible d'ouvrir l'éditeur de cues : { $error }
ui-track-detail-waiting-for-usb-job = Attente de la fin de la tâche USB en cours avant d'enregistrer les cues...
ui-track-detail-max-cues = 8 points de cue au maximum.
ui-track-detail-cue-already-there = Il y a déjà un cue à cet endroit.

## ui-track-columns

ui-track-columns-button-tooltip = Afficher, masquer et réordonner les colonnes
ui-track-columns-blocked-order-edit = Enregistrez ou annulez d'abord la modification de l'ordre pour changer les colonnes
ui-track-columns-title = Colonnes
ui-track-columns-intro = Cochez les colonnes à afficher et faites-les glisser dans l'ordre voulu. Toutes les listes de morceaux utilisent cette disposition.
ui-track-columns-reset = Rétablir par défaut
ui-track-columns-done = Terminé

## ui-track-row

ui-track-row-edit-cues-aria-label = Points de cue et beatgrid
ui-track-row-empty = Aucun morceau disponible.
ui-track-row-stop = Arrêter
ui-track-row-analyze = Analyser
ui-track-row-analyze-tooltip = Ajouter la forme d'onde, le BPM, la tonalité et le premier temps manquants ; un BPM ou une tonalité modifiés ou importés sont conservés
ui-track-row-reanalyze-tooltip = Remettre la forme d'onde, le BPM, la tonalité et le premier temps à une nouvelle analyse, en remplaçant les valeurs importées ou modifiées (les cues sont conservés)
ui-track-row-edit-cues-tooltip = Modifier les points de cue et la beatgrid
ui-track-row-edit-cues-analyze-first-tooltip = Analysez d'abord ce morceau
ui-track-row-add-to-tooltip = Ajouter à { $playlist }
ui-track-row-add-no-playlist-tooltip = Créez et activez d'abord une playlist, puis ajoutez-y des morceaux.
ui-track-row-remove-from-tooltip = Retirer de { $playlist }
ui-track-row-waveform-preview-aria-label = aperçu de la forme d'onde

## ui-track-table

ui-track-table-album = Album
ui-track-table-bpm = BPM
ui-track-table-fmt = Fmt
ui-track-table-genre = Genre
ui-track-table-key = Ton.
ui-track-table-length = Durée
ui-track-table-library-tracks-aria-label = Morceaux de la bibliothèque
ui-track-table-playlist-tracks-aria-label = Morceaux de la playlist
ui-track-table-select-all-aria-label = Tout sélectionner
ui-track-table-sort-hint = trié par →
ui-track-table-sort-artist = Artiste
ui-track-table-sort-title = Titre
ui-track-table-total-time = Durée totale : 0:00
ui-track-table-track = Morceau
ui-track-table-usb-history-tracks-aria-label = Morceaux de l'historique USB
ui-track-table-usb-playlist-tracks-aria-label = Morceaux de la playlist USB
ui-track-table-year = Année

ui-track-table-stop-failed = Échec de l'arrêt : { $error }

## ui-tracklist-export

ui-tracklist-export-estimated-note = Les heures sont estimées en additionnant les durées des morceaux dans l'ordre de lecture : les CDJ n'enregistrent pas l'heure de lecture de chaque morceau sur la clé USB.
ui-tracklist-export-include-estimated-times = Inclure les heures estimées
ui-tracklist-export-ok = Exporter…
ui-tracklist-export-placement = Position de l'heure
ui-tracklist-export-placement-after = Après le morceau (Artiste - Titre - 0:00)
ui-tracklist-export-placement-before = Avant le morceau (0:00 Artiste - Titre)
ui-tracklist-export-start-from = Commencer au morceau
ui-tracklist-export-title = Exporter la tracklist

## ui-usb

ui-usb-apply-repairs = Appliquer les corrections
ui-usb-audio-formats = Formats audio
ui-usb-backups-tooltip = Lister, restaurer et supprimer les sauvegardes des bases de données de cette clé USB
ui-usb-diag-back-to-report = Retour au rapport
ui-usb-diag-log-link = → journal des événements
ui-usb-drag-to-set-tooltip = Glisser pour définir l'ordre de préférence
ui-usb-edb = eDB
ui-usb-forget-tooltip = Supprimer cette clé USB de cet ordinateur
ui-usb-health-card = État et diagnostics
ui-usb-health-unknown = État de la clé USB : inconnu
ui-usb-empty-heading = Branchez une clé USB pour la parcourir et exporter
ui-usb-init-hint = Aucune structure External library trouvée.
ui-usb-initialize-usb = Initialiser la structure USB
ui-usb-issues = Problèmes
ui-usb-matched = Concordants
ui-usb-parity-intro = Vérifie que les anciens et les nouveaux lecteurs Pioneer voient les mêmes playlists et les mêmes morceaux sur cette clé USB.
ui-usb-parity-report = Rapport de parité
ui-usb-parity-report-tooltip = Vérifier que l'ancienne (PDB) et la nouvelle (eDB) base de données de la clé USB concordent
ui-usb-pdb = PDB
ui-usb-playlist = Playlist
ui-usb-preview-repairs = Aperçu des corrections
ui-usb-re-diagnose = Rediagnostiquer la clé USB
ui-usb-recent = Clés USB récentes
ui-usb-repair-technical-details = Détails techniques
ui-usb-root-path = Aucune clé USB sélectionnée
ui-usb-select-usb-folder = Choisir le dossier USB
ui-usb-status = État
ui-usb-title = Navigateur USB

ui-usb-auto-diagnostics-failed = Échec des diagnostics automatiques : { $error }
ui-usb-diagnostics-running = Diagnostics de la clé USB en cours...
ui-usb-export-running = Export de { $name } vers la clé USB...
ui-usb-export-playlists-running = { $count ->
    [one] Export de { $count } playlist vers la clé USB...
   *[other] Export de { $count } playlists vers la clé USB...
}
ui-usb-export-starting = Démarrage de l'export USB...
ui-usb-export-failed = Échec de l'export : { $error }. Voir le Journal des événements pour les détails.
ui-usb-export-failed-unknown = Échec de l'export USB
ui-usb-forget-path-aria-label = Supprimer { $path }
ui-usb-parity-running = Rapport de parité USB en cours...
ui-usb-repair-applying = Application des corrections USB prises en charge...
ui-usb-repair-previewing = Aperçu des corrections USB...

## ui-usb-history

ui-usb-history-export-tracklist = Exporter la tracklist dans un fichier texte
ui-usb-history-load-failed = Impossible de charger l'historique USB : { $error }
ui-usb-history-loading = Chargement de l'historique USB...
ui-usb-history-search-placeholder = Rechercher dans les morceaux de l'historique
ui-usb-history-title = Historique USB

## ui-usb-player-menu

ui-usb-player-menu-add = Ajouter →
ui-usb-player-menu-available = Disponibles (masqués)
ui-usb-player-menu-available-aria-label = Éléments masqués disponibles du menu du lecteur
ui-usb-player-menu-background-colour = Couleur de fond
ui-usb-player-menu-background-colour-aria-label = Couleur de fond du lecteur
ui-usb-player-menu-current = Menu actuel du lecteur
ui-usb-player-menu-current-aria-label = Éléments visibles actuels du menu du lecteur
ui-usb-player-menu-down = Descendre
ui-usb-player-menu-loading = Chargement de la configuration du menu du lecteur...
ui-usb-player-menu-remove = ← Retirer
ui-usb-player-menu-title = Éditeur du menu du lecteur
ui-usb-player-menu-up = Monter

## ui-usb-playlists

ui-usb-playlists-compared = Playlists comparées
ui-usb-playlists-edit-order = Modifier l'ordre
ui-usb-playlists-edit-order-tooltip = Changer l'ordre des playlists et des morceaux qu'elles contiennent ; rien n'est écrit avant « Enregistrer l'ordre »
ui-usb-playlists-save-order = Enregistrer l'ordre
ui-usb-playlists-order-edited = Ordre des morceaux modifié
ui-usb-playlists-drag-search-active = Effacez la recherche pour faire glisser des morceaux
ui-usb-playlists-leave-title = Ordre non enregistré
ui-usb-playlists-leave-message = Le nouvel ordre des playlists de cette clé USB n'est pas encore enregistré.
ui-usb-playlists-leave-discard = Abandonner les modifications
ui-usb-playlists-leave-stay = Rester
ui-usb-playlists-load-failed = Impossible de charger la playlist USB : { $error }
ui-usb-playlists-loading = Chargement des playlists USB...
ui-usb-playlists-read-failed = Échec du chargement de la clé USB
ui-usb-playlists-reading = Lecture de la base de données USB...
ui-usb-playlists-remove-title = Retirer la playlist USB
ui-usb-playlists-search-placeholder = Rechercher dans les morceaux de la playlist USB
