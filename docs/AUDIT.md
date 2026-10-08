# Audit correctif des 7 et 8 octobre 2026

Travail incomplet. Compteur d'audits complets consécutifs sans problème : **0/2**.
Les corrections et les validations en échec ont remis le compteur à zéro.
Aucune passe n'est comptée comme complète tant que les contrôles Windows
nécessaires restent impossibles et que le banc audio reste en échec.

Le dépôt était propre au départ. Aucun fichier AGENTS.md ou CLAUDE.md n'a été
trouvé. README, PORTING, documentation des actions, du banc et des démos,
Cargo et les quatre workflows ont servi à identifier les comportements et
contrôles attendus. Les fichiers applicatifs, tests, exemples, scripts,
descripteurs et configurations ont été relus au-delà du diff. L'intégration
de whisper-rs-sys et son correctif local de lecture des dimensions ont été
examinés ; cela ne constitue pas une revue exhaustive du C/C++ amont vendorié.

## Corrections appliquées

| Zone | Défaut et correction |
| --- | --- |
| Audio et données | Erreurs d'écriture perdues à l'arrêt et erreurs de synchronisation d'un ancien enregistrement : lecture sous le même verrou et identification de l'enregistrement. Fermeture du fichier temporaire avant nettoyage sous Windows. Protection des manifestes lors d'un renommage limité à la casse ou d'un échec de suppression. |
| Sécurité | WASAPI 0.24 concerné par RUSTSEC-2026-0332 remplacé par 0.25. Trames IPC tronquées refusées ; titres trop longs refusés par la CLI. Permissions du pipe Windows restreintes au propriétaire et à LocalSystem. Messages non interprétés comme du markup. |
| Interactions UI | Vérification de l'état après les dialogues d'import ; sauvegarde avant modification de l'état ; rejet des résultats asynchrones d'une autre réunion ou révision ; restauration des textes lors d'échecs de sauvegarde. Actions de paragraphe visibles au clavier et colonne d'actions défilable sur les écrans bas. |
| Ressources et délais | Références faibles des callbacks du lecteur pour libérer les réunions fermées. Job Objects pour les outils audio Windows, incluant lecture, import, export, décodage, sondes et formes d’onde. Sondes d'agents limitées en temps et en taille ; arrêt des descendants avant d'attendre l'envoi du texte. |
| Entrées et configuration | Horodatages et couleurs invalides sans panic ni débordement ; caractères de contrôle filtrés des noms. Lecture des paramètres racine avec guillemets, chemins Windows et caractères # préservés. Chemins des actions rendus absolus avant changement de répertoire. |
| Outillage et documentation | Exemple compilable avec all-targets et corrections des diagnostics des contrôles existants. Échecs WiX bloquants, version MSI issue de Cargo, schémas GTK limités au processus. Suivi Cargo de GGML_NATIVE et des options CMake transmises, pour éviter un cache de compilation obsolète. Banc utilisant le vrai nom du binaire et refusant erreurs ou mesures absentes. Installation source et limites documentées corrigées. Notes Obsidian : échappement des noms en YAML et écriture UTF-8 explicite. |

## Validations réellement exécutées

| Contrôle | Résultat |
| --- | --- |
| cargo fmt --all -- --check | Réussi. |
| cargo check --locked --all-targets | Réussi sur Linux. |
| cargo clippy --locked --all-targets -- -D warnings | Réussi sur le code applicatif Linux ; six avertissements non bloquants proviennent du build script et des bindings de whisper-rs-sys. |
| cargo test --locked | 74 tests réussis sur Linux, dont deux nouveaux tests de drainage des sorties et de fermeture des pipes conservés par un descendant. |
| cargo build --locked ; build --release --locked avec GGML_NATIVE=OFF | Réussis sur Linux. |
| python3 bench/tests.py | 18 tests réussis. |
| cargo audit | 278 dépendances ; aucune vulnérabilité connue ni avertissement signalé par la base RustSec consultée le 7 octobre 2026. |
| Cache C++ | Changement effectif de GGML_NATIVE détecté par Cargo ; build script relancé et contrôle de types réussi. |
| Syntaxe des fichiers | 11 scripts Python, 4 workflows YAML, 9 JSON, 3 XML et 3 scripts shell validés. Analyse syntaxique PowerShell 7.6.6 de 39 scripts de packaging, CI et exemples README réussie ; pas d'exécution du packaging Windows. |
| Interface GTK sous Xvfb | Ouverture d'une réunion synthétique, édition, suppression, Undo, retour à une nouvelle réunion et inspection du rendu. Vérification de la petite fenêtre après correction du défilement ; nouvelle ouverture après confinement des outils audio, durée détectée et actions accessibles à 820 × 560. Aucun test matériel WASAPI. |
| Exemple Obsidian | Exécution isolée avec guillemets, antislashs et Unicode ; les propriétés YAML et le texte restent intacts. |
| CLI | Rejet effectif des titres IPC trop longs et des nombres de locuteurs invalides. |
| Banc audio complet des six fixtures | **5/6 réussies** avec les modèles par défaut et les seuils existants. Échec import : erreur de locuteur **0,078869**, maximum **0,05**. Tous les locuteurs sont retrouvés ; l'écart provient surtout de couverture temporelle manquante, dont une première intervention non détectée. Le seuil est conservé. |
| Vérification de types isolée Windows GNU | Réussie pour capture, IPC, confinement des processus, export, widget conditionnel et source PCM du lecteur, avec leurs dépendances Windows. Ce contrôle utilise un petit harness avec des substituts de chemins/réglages et ne remplace pas la compilation de l'application GTK complète. |

Le premier banc audio a échoué au téléchargement avec UnknownIssuer : le
client TLS de l'application ne connaît pas le certificat du proxy de cette
session. Le banc avec modèles disponibles a ensuite été exécuté dans un
cache isolé après téléchargement via les certificats système approuvés et
vérification des SHA-256 LFS des révisions figées. TLS n'a pas été désactivé.
Résultats et transcriptions : /tmp/meeting-audit-bench/.

La reprise du 8 octobre a comparé le prétraitement et le cache Nemotron avec
l’implémentation Transformers de référence, puis mesuré les probabilités du
modèle sur le PCM exact de la fixture import. La première réponse de Ben est
très faible en niveau sonore et en probabilité de locuteur ; les annotations
incluent également les silences de fin des clips Piper. Aucune correction de
ces annotations ou de l’algorithme n’a été validée. Les seuils restent inchangés.
La répétition des six scénarios après le confinement des outils audio obtient
exactement les mêmes scores, avec le seul échec import (5/6 réussis). Les deux
résultats sont conservés dans results.json et results-continuation.json.
La correction ultérieure du suivi des options de compilation ne modifie pas
l’algorithme audio ; son changement GGML_NATIVE=OFF vers off a déclenché
une recompilation Cargo effective avec un check réussi, avant restauration
explicite de GGML_NATIVE=OFF.

Les fonctions du widget Omarchy inutilisées sous Windows sont désormais
compilées uniquement sur la plateforme concernée (ou pour leurs tests). Cela
supprime le diagnostic Windows confirmé sans désactiver de contrôle.

## Problèmes et contrôles restants

- Résoudre l'échec de couverture de diarisation sur import avec une cause et
  une correction validées ; aucun ajustement arbitraire des seuils ou du
  modèle n'a été appliqué pour obtenir un résultat vert.
- Le check, les 80 tests et le build release Windows ont réussi dans le run
  `37776726518`, y compris les tests de confinement, de pipe privé et de
  manifestes. Revalider sur l'état corrigé les contrôles devenus bloquants,
  le nouveau downmix, la simulation audio, le rendu GTK et le packaging MSI.
  L'accès IPC entre deux comptes et la mise à jour MSI restent à vérifier.
- Les captures et lectures WASAPI matérielles ne sont pas autorisées sur le
  PC personnel ; la simulation ne valide pas les pilotes ni le hotplug.
- L'intégration Quickshell/Omarchy, Vulkan et les sources C/C++ amont n'ont pas
  reçu une validation exhaustive dans cet environnement.
- Après ces contrôles et corrections, reprendre deux audits complets sans
  modification sur le même état, avec une seconde relecture des interactions.

## Reprise nécessaire sur Windows

Utiliser les corrections présentes dans le répertoire de travail, et non un
ancien commit. Le workflow `.github/workflows/windows.yml` prépare Rust GNU,
GTK/MSYS2, FFmpeg et ONNX Runtime puis exécute formatage, tests Python,
`cargo check --locked`, `cargo test --locked`, Clippy, build release et contrôle
d’installation MSI. Une exécution sur Windows de cet état, accompagnée des
parcours UI et audio synthétiques décrits plus bas, est nécessaire pour lever le blocage.
Une exécution de ce workflow sur la branche de test est maintenant en cours.

La phase locale précédente n’avait effectué aucun commit ni push. La reprise
CI a publié une branche de test distincte basée sur `ci/runners-locaux`, avec
les corrections de l’audit et la simulation audio ; aucune release ni aucun
déploiement n’est demandé.

Branche : `audit/windows-synthetic-audio-20261008`. Premier run sécurisé :
https://github.com/Rhadamanthe0/meeting-recorder-windows/actions/runs/37771939044.
La relecture du simulateur a ensuite corrigé la lecture simultanée des pistes
(une sortie silencieuse indépendante par piste, comme dans le lecteur normal).

## Reprise CI sans périphériques réels

Le runner `PC-PRO-CLEM-meeting-recorder` a été identifié dans le job Windows.
Les runs 37769817474 et 37769825072 (y compris ses nouvelles tentatives) ont été annulés avant le lancement GUI,
car le workflow existant ouvrait le binaire normal sans arguments. La feature
`ci-audio` produit maintenant deux entrées générées en mémoire et une sortie
silencieuse, avec identifiants d’application, pipe et préférences isolés.
L’installation du MSI est déplacée sur `windows-latest` pour préserver le PC.
Voir [les contrôles et leurs limites](ci-audio-windows.md).

Contrôles locaux : 74 tests Rust Linux (normal et ci-audio), Clippy Linux,
contrôle de types Windows isolé, garde des spawns exécutée sous PowerShell et
36 blocs/scripts PowerShell analysés avec succès. Les résultats natifs et le
rendu Windows restent à obtenir. Compteur inchangé : **0/2**.

La CI a été relancée sur le commit `cb2d50e` dans le run
https://github.com/Rhadamanthe0/meeting-recorder-windows/actions/runs/37773593050.
Le premier run sécurisé avait réussi les gardes, le formatage et les 18 tests
Python avant son remplacement. La relecture ultérieure rétablit le diagnostic
des DLL installées sur la machine jetable avec un outil explicitement fourni
et un échec bloquant si le contrôle est impossible ; elle rend aussi explicite
l’attente et la capture de sortie des sondes de version du binaire GUI.
La syntaxe PowerShell/YAML de ces corrections a été vérifiée localement ;
leur exécution native reste à obtenir.

Le run `37773593050` a ensuite réussi `cargo check --locked` sur l'application
Windows complète et exécuté 80 tests : **79 réussis, 1 en échec**. Le test
de rollback supposait que l'attribut lecture seule empêcherait la suppression,
ce qui n'est pas le cas sur ce runner. Il utilise maintenant un handle
Windows qui refuse le partage pour suppression et vérifie aussi le titre
effectivement rouvert. Le test et les assertions de rollback sont conservés.
Les tests natifs de confinement des descendants, du pipe privé et du renommage
limité à la casse ont réussi. Build release, MSI, audio synthétique et rendu
GUI n'ont pas encore été exécutés. Compteur inchangé : **0/2**.

Le run `37776726518` (commit `fb174b8`) confirme désormais les **80 tests
Windows réussis**, le check complet et le build release portable. La lecture
des journaux révèle aussi des diagnostics Clippy Windows (étape encore
informative) et l'absence de Chocolatey bloquant WiX. Les diagnostics sont
corrigés sans désactivation de lint : code Unix limité à sa plateforme,
attente du processus fixture dans un thread, variantes de lecteur proportionnées
et formes Rust demandées par les contrôles existants. Les métadonnées
de l'endpoint sont conservées dans les diagnostics audio existants. WiX
3.14.1 est désormais fourni par une archive officielle portable vérifiée
(taille 41 297 555, SHA-256 6ac824e1642d6f7277d0ed7ea09411a508f6116ba6fae0aa5f2c7daa2ff43d31),
sans Chocolatey ni installation globale. Les tests audio synthétiques, leur
Clippy, le rendu et le MSI restent à exécuter sur cet état corrigé.

La relecture du convertisseur confirme également une perte du canal arrière
droit en quadriphonie : l'indice 3 était toujours traité comme un LFE. Le
convertisseur utilise désormais le masque réel WAVEFORMATEXTENSIBLE, garde
les canaux arrière/latéraux/hauts sur leur côté et n'omet que le vrai LFE.
Un test traverse le convertisseur avec quad, surround et 3.1 : il échoue sur
l'ancienne implémentation et réussit sur la nouvelle. Les cinq tests purs
de downmix/resampling extraits du code Windows ont été exécutés sur Linux
avec succès ; le contrôle de types et Clippy Windows isolés réussissent.
Le test de lecture synthétique couvre aussi le décodeur de repli Rodio, et
Clippy devient bloquant dans le workflow normal et dans la variante ci-audio.
La nouvelle validation native reste à exécuter. Compteur : **0/2**.

Le run `37784836247` (commit `d13a490`) est incomplet : le service Windows
ne trouve pas `pwsh` dans son PATH avant les validations. Le propriétaire
confirme que le runner fonctionne comme service. Le workflow prépare maintenant
PowerShell depuis Windows PowerShell 5.1, réutilise son installation si présente
ou fournit une archive officielle portable avec taille/digest vérifiés. Le rendu
et le pipe GUI sont déplacés dans `verify-gui` sur une VM Windows GitHub ;
le runner personnel ne lance jamais l’interface. YAML, séparation des jobs,
41 blocs/scripts PowerShell et `git diff --check` réussissent localement.
La nouvelle validation native reste à obtenir ; compteur **0/2**.

Le run `37785634562` a réussi la préparation PowerShell/Rust, MSYS2/GTK,
FFmpeg/ORT, les gardes, le formatage et les tests Python, puis `cargo check`
a été interrompu à 13:46 UTC. Le propriétaire confirme avoir redémarré
le service du runner et indique qu'il est à nouveau actif. Le contrôle est
incomplet et ne compte pas comme réussi. L'audit des 278 dépendances relancé
le 8 octobre réussit, sans vulnérabilité connue signalée. Les commentaires
de CI ne donnent plus une version Cargo 0.1.11 obsolète (le dépôt est 0.1.13).

Après le redémarrage, l'annulation forcée a clôturé le job interrompu.
Le même commit `c70c58a` a été lancé sur la branche distincte
`audit/windows-synthetic-audio-service-20261008` (run `37787766647`) pour
contourner temporairement le verrou du premier run. Ce run a échoué pendant
l'installation MSYS2 avant toute compilation. Le journal montre un profil
`Système` et des erreurs `Illegal byte sequence` dans fontconfig. Une locale
UTF-8 explicite est maintenant fournie aux builds Windows et release ;
son effet sur la préparation native reste à confirmer.

La relecture confirme aussi les alias Windows réservés `COM¹`, `COM²`,
`COM³`, `LPT¹`, `LPT²` et `LPT³`, documentés par Microsoft mais oubliés par
le filtre. Ils empêchaient de sauvegarder un manifeste portant ce titre.
Le test existant est étendu aux six alias et à une extension : il échoue
sur l'ancien code (`COM¹ -> COM¹`) puis réussit après correction, sans
ouvrir aucun périphérique. Les 74 tests Linux, Clippy ci-audio et le
formatage réussissent sur cet état. Compteur **0/2**.

Le run `37790627534` (`5c26bbd`) échoue encore avant les validations Rust :
la signature UCRT64 est invalide, puis la base est verrouillée. La locale
UTF-8 seule n'a donc pas levé le blocage. La lecture du code de l'action
MSYS2 pinnée confirme également un `taskkill /F /FI MODULES eq msys-2.0.dll`
global durant sa mise à jour, susceptible d'arrêter les terminaux du PC.
La préparation utilise maintenant le miroir officiel principal et force
UCRT64 dès son initialisation (MINGW64 était hérité dans le journal).
La mise à jour complète et l'installation des mêmes paquets sont conservées
dans un script séparé qui ferme seulement l'agent du keyring MSYS2 temporaire
et attend la sortie des processus qu'il lance. Toutes les signatures et
les codes d'erreur restent bloquants ; aucune clé ni vérification n'est supprimée.
Le chemin d'installation exposé vient directement de l'action, sans deviner
le dossier du runner. L'exécution native reste à obtenir. Compteur **0/2**.

Le run `37792349599` (`2555b13`) réussit l'initialisation MSYS2, les mises à
jour complètes et l'installation GTK/UCRT64 sans arrêt global de processus.
Les gardes, rustfmt et les 18 tests Python réussissent également. Windows
refuse ensuite de lancer `C:\ProgramData\cargo\bin\cargo.exe` pour
`cargo check` (« Aucune application n'est associée au fichier spécifié »).
Il ne s'agit pas d'un diagnostic de compilation Rust. Le run `37795814144`
(`c32abcc`) conserve le contrôle bloquant et ajoute le chemin, la taille,
l'empreinte SHA-256 et la chaîne d'exceptions du binaire réellement lancé.
Son résultat reste à obtenir. Les validations locales relancées sur cet état
réussissent : fmt, check all-targets, Clippy ci-audio all-targets, 74 tests
Rust ci-audio, 18 tests Python, quatre YAML, 44 blocs/scripts PowerShell et
`git diff --check`. Des avertissements de bindings générés de whisper-rs-sys
subsistent ; aucun avertissement du code principal n'est accepté par Clippy.
Compteur **0/2**, notamment en raison du banc audio import encore en échec.

Le run `37795814144` lance effectivement Cargo, mais compile pour
`x86_64-pc-windows-msvc` et sélectionne Visual Studio, alors que Rust GNU
avait été annoncé avant la restauration du cache. La préparation GTK reste
correcte ; Whisper échoue avec MSBuild/FTK1011. Le cache restaurait aussi
`CARGO_HOME/bin`, avec un `cargo.exe` lié (taille de lien nulle mais empreinte
de contenu non vide). Les workflows Windows et release excluent désormais
les exécutables du cache et utilisent un nouvel espace de clés, pour ne pas
extraire les anciennes archives qui les contiennent encore. La toolchain GNU
est fixée dans l'environnement et les hôtes effectifs de Rustc et Cargo sont
vérifiés après restauration, avant la longue préparation GTK.
Les tests/Clippy synthétiques passent avant le packaging, pour qu'une erreur
MSI ne les empêche plus de tourner ; seule la compilation release synthétique
reste après la fabrication du MSI normal. Le contrôle PE reste bloquant mais
n'affiche que le subsystem pertinent : son dump complet avait produit plus
de 40 Mo de logs. Nouvelle validation native nécessaire. Compteur **0/2**.
