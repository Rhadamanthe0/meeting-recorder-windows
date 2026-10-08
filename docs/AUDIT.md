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
- La compilation complète Windows GNU tentée s'arrête dans glib-sys :
  environnement pkg-config/GTK Windows absent. Exécuter sur Windows les
  contrôles du projet, les nouveaux tests de permissions IPC et de manifestes,
  ainsi que l'accès IPC entre deux comptes, la capture/lecture WASAPI et les
  parcours GTK. Vérifier installation, mise à jour et désinstallation MSI.
- Valider sur Windows la fermeture sur crash des outils ffmpeg.exe et
  ffprobe.exe désormais confinés. Les tests Linux du helper et la vérification
  de types Windows isolée réussissent ; ce n’est pas une preuve de comportement
  natif Windows.
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
parcours matériels et UI décrits plus haut, est nécessaire pour lever le blocage.
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
Les runs 37769817474 et 37769825072 ont été annulés avant le lancement GUI,
car le workflow existant ouvrait le binaire normal sans arguments. La feature
`ci-audio` produit maintenant deux entrées générées en mémoire et une sortie
silencieuse, avec identifiants d’application, pipe et préférences isolés.
L’installation du MSI est déplacée sur `windows-latest` pour préserver le PC.
Voir [les contrôles et leurs limites](ci-audio-windows.md).

Avant exécution native : 74 tests Rust Linux (normal et ci-audio), Clippy Linux,
contrôle de types Windows isolé, garde des spawns exécutée sous PowerShell et
36 blocs/scripts PowerShell analysés avec succès. Les résultats natifs et le
rendu Windows restent à obtenir. Compteur inchangé : **0/2**.
