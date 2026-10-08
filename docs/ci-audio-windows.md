# Tests Windows sans accès aux périphériques audio

Le workflow `windows` utilise le runner `[self-hosted, windows, x64,
meeting-recorder]`. Les PR de forks restent sur `windows-latest`.

Si le runner personnel est indisponible, le lancement manuel du workflow
avec `hosted=true` utilise une VM Windows GitHub pour le build et les mêmes
contrôles. Ce choix est séparé de l'exécution locale dans la concurrence CI.

Les tests ordinaires n'ouvrent pas de périphérique audio. Le MSI produit est
le binaire normal. Son installation et sa désinstallation sont vérifiées dans
le job `verify-msi`, sur une machine GitHub jetable, pour ne pas remplacer une
installation existante sur le PC. La publication du workflow `release` dépend
également de cette vérification.

## Entrées et sortie simulées

`cargo test --locked --features ci-audio` et
`cargo build --release --locked --features ci-audio` construisent un binaire
réservé aux tests. Sur Windows :

- le microphone est un ton généré de 440 Hz, l'audio système un ton de 880 Hz ;
- ces paquets mono float32 à 44,1 kHz traversent le convertisseur et le chemin
  d'enregistrement réels (48 kHz stéréo, pause, reprise, flush et arrêt) ;
- la lecture décode les fichiers vers un sink silencieux en mémoire ;
  aucun `OutputStream` matériel n'est créé ;
- aucun client WASAPI, périphérique par défaut ou périphérique sauvegardé
  n'est ouvert ; aucun pilote virtuel n'est installé ;
- l'identifiant d'application, le pipe IPC et les dossiers de configuration
  portent un nom distinct, pour ne pas commander une application normale
  déjà ouverte ou réutiliser ses préférences.

Le choix est fixé à la compilation : aucune variable d'environnement ni
préférence ne permet un repli matériel dans ce binaire. `--version` annonce
`CI synthetic audio; hardware audio disabled on Windows`. La version normale,
sans cette feature, conserve son comportement audio habituel.

## Contrôles effectués

Les tests spécifiques vérifient l'écriture de samples non nuls, la suspension
des écritures pendant une pause, la reprise et la libération de l'entrée ;
ils vérifient aussi le décodage et la fin d'une lecture sans sortie matérielle.

Le job `verify-gui` s'exécute sur une VM GitHub Windows jetable, car le runner
du PC fonctionne comme service sans bureau interactif. Il reçoit le runtime
synthétique construit par le runner. Le test GUI n'exécute que ce binaire après vérification de sa
version. Il vérifie son pipe isolé et capture seulement sa fenêtre via
`PrintWindow`, jamais l'écran complet. Il n'envoie aucune touche ni aucun clic
global au bureau. Ses journaux, état IPC et image figurent dans l'artefact
`synthetic-audio-results`.

Si la VM ne permet pas le rendu d'une fenêtre, ce contrôle échoue explicitement.
La simulation ne valide pas les échanges WASAPI avec un pilote, les changements
de périphérique ou la qualité acoustique réelle. Ces contrôles restent distincts.
L'échec connu du banc de diarisation `import` n'est pas corrigé par ce mode.
