# Audit correctif des 7 et 8 octobre 2026

Travail incomplet. Compteur d'audits complets consécutifs sans problème : **0/2**.
Les corrections et les validations en échec ont remis le compteur à zéro.
Les contrôles Windows sur VM sont désormais réussis. Aucune passe n'est
comptée comme complète tant que le banc audio reste en échec.

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
| Ressources et délais | Références faibles des callbacks du lecteur pour libérer les réunions fermées. Déconnexion des clients IPC Windows qui ne lisent plus, pour libérer les workers et handles bloqués. Job Objects pour les outils audio Windows, incluant lecture, import, export, décodage, sondes et formes d’onde. Sondes d'agents limitées en temps et en taille ; arrêt des descendants avant d'attendre l'envoi du texte. |
| Entrées et configuration | Horodatages et couleurs invalides sans panic ni débordement ; caractères de contrôle filtrés des noms. Lecture des paramètres racine avec guillemets, chemins Windows et caractères # préservés. Chemins des actions rendus absolus avant changement de répertoire. |
| Outillage et documentation | Exemple compilable avec all-targets et corrections des diagnostics des contrôles existants. Échecs WiX bloquants, version MSI issue de Cargo, schémas GTK limités au processus. Suivi Cargo de GGML_NATIVE et des options CMake transmises, pour éviter un cache de compilation obsolète. Banc utilisant le vrai nom du binaire et refusant erreurs ou mesures absentes. Installation source et limites documentées corrigées. Notes Obsidian : échappement des noms en YAML et écriture UTF-8 explicite. |

## Validations réellement exécutées

| Contrôle | Résultat |
| --- | --- |
| cargo fmt --all -- --check | Réussi. |
| cargo check --locked --all-targets | Réussi sur Linux. |
| cargo clippy --locked --all-targets -- -D warnings | Réussi sur le code applicatif Linux ; six avertissements non bloquants proviennent du build script et des bindings de whisper-rs-sys. |
| cargo test --locked | 75 tests réussis sur Linux, dont les tests de drainage des sorties et de fermeture des pipes conservés par un descendant ou un client qui ne lit plus. |
| cargo build --locked ; build --release --locked avec GGML_NATIVE=OFF | Réussis sur Linux. |
| python3 bench/tests.py | 18 tests réussis. |
| cargo audit | 278 dépendances ; aucune vulnérabilité connue ni avertissement signalé par la base RustSec actualisée le 8 octobre 2026. |
| Cache C++ | Changement effectif de GGML_NATIVE détecté par Cargo ; build script relancé et contrôle de types réussi. |
| Syntaxe des fichiers | 11 scripts Python, 4 workflows YAML, 9 JSON, 3 XML et 3 scripts shell validés. Analyse syntaxique PowerShell 7.6.6 réussie, avec les scripts CI ajoutés et les gardes GNU. |
| Interface GTK sous Xvfb | Ouverture d'une réunion synthétique, édition, suppression, Undo, retour à une nouvelle réunion et inspection du rendu. Vérification de la petite fenêtre après correction du défilement ; nouvelle ouverture après confinement des outils audio, durée détectée et actions accessibles à 820 × 560. Aucun test matériel WASAPI. |
| Exemple Obsidian | Exécution isolée avec guillemets, antislashs et Unicode ; les propriétés YAML et le texte restent intacts. |
| CLI | Rejet effectif des titres IPC trop longs et des nombres de locuteurs invalides. |
| Banc audio complet des six fixtures | **5/6 réussies** avec les modèles par défaut et les seuils existants. Échec import : erreur de locuteur **0,078869**, maximum **0,05**. Tous les locuteurs sont retrouvés ; l'écart provient surtout de couverture temporelle manquante, dont une première intervention non détectée. Le seuil est conservé. |
| Vérification de types isolée Windows GNU | Réussie pour capture, IPC, confinement des processus, export, widget conditionnel et source PCM du lecteur, avec leurs dépendances Windows. Ce contrôle utilise un petit harness avec des substituts de chemins/réglages et ne remplace pas la compilation de l'application GTK complète. |
| Application Windows complète, commit 799a6e8 | Sur VM GitHub : check, **83 tests ordinaires**, **85 tests ci-audio**, Clippy bloquant dans les deux configurations, release et subsystem GUI réussis. Les tests de libération des I/O d'un client IPC lent et de conversion FC/LFE réussissent dans les deux suites. |
| Packaging Windows | Construction du MSI normal, installation par utilisateur, vérification des imports et de --help sans UCRT au PATH, FFmpeg embarqué et désinstallation réussis sur VM GitHub. |
| Interface Windows synthétique | Marqueur de sécurité du binaire, pipe CI isolé et état idle vérifiés. Capture de la seule fenêtre de test inspectée : textes, champs, boutons et indicateurs visibles, aucun rendu vide ni problème de mise en page identifié. |

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
- Les contrôles Windows, le downmix, la simulation audio, le rendu GTK et
  le packaging MSI réussissent sur `799a6e8` dans le run `37808502722`.
  L'accès IPC entre deux comptes et la mise à jour d'une installation MSI
  existante n'ont pas été exercés.
- Les captures et lectures WASAPI matérielles ne sont pas autorisées sur le
  PC personnel ; la simulation ne valide pas les pilotes ni le hotplug.
- L'intégration Quickshell/Omarchy, Vulkan et les sources C/C++ amont n'ont pas
  reçu une validation exhaustive dans cet environnement.
- Après ces contrôles et corrections, reprendre deux audits complets sans
  modification sur le même état, avec une seconde relecture des interactions.

## Historique de la reprise Windows

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

Le run `37798798983` (`4553ef8`) reste bloqué dans « Set up job » sur
`PC-PRO-CLEM-meeting-recorder`, avant toute commande du dépôt ; aucun journal
n'est disponible à ce stade. Une option de dispatch `hosted=true` permet
maintenant d'exécuter exactement le même build sur une VM Windows GitHub,
dans un groupe de concurrence distinct. Elle ne change pas le runner des
push/PR internes habituels et ne modifie pas la configuration du PC.

La CI sur VM `37799865905` (`14500f8`) réussit complètement : hôtes GNU
effectifs, check, **81 tests Windows ordinaires**, Clippy bloquant,
**83 tests ci-audio**, Clippy ci-audio, release portable, subsystem GUI,
MSI normal, compilation release synthétique, rendu/IPC synthétiques et
installation/désinstallation du MSI sans UCRT au PATH. Les tests d'entrée
synthétique et de décodage vers une sortie silencieuse ont effectivement
tourné et réussi. Le contrôle du downmix quad/surround réussit aussi.
Le téléchargement de l'image est refusé par le proxy de l'environnement
d'audit (HTTP 403), même après récupération via le connecteur GitHub.
Le script joint donc maintenant une copie bornée de la fenêtre de test
dans ses logs ; l'artefact PNG demeure conservé.

La relecture trouve un doublon `new-window` dans `--help` : une seule ligne
est conservée avec l'information Ctrl+N. Le binaire Linux recompilé confirme
ce résultat. Elle confirme aussi qu'une file de statut pleine retirait son
sender sans interrompre les I/O synchrones Windows ; un client connecté qui
ne lit plus pouvait donc conserver des threads et des handles indéfiniment.
Un handle de serveur par client permet maintenant de déconnecter cette
instance quand elle est retirée. Le nouveau test garde le handle du pair
ouvert et sans lecture, puis exige la fin des deux workers après retrait.
Check et Clippy Windows croisés, check/Clippy Linux, les 74 tests Rust Linux
et les 18 tests Python réussissent.

La CI native finale `37803329704` sur `db3ef2d` réussit entièrement :
https://github.com/Rhadamanthe0/meeting-recorder-windows/actions/runs/37803329704.
Les **82 tests Windows ordinaires** et **84 tests ci-audio** passent, ainsi
que Clippy, les deux builds release, le MSI, son installation/désinstallation
et le contrôle GUI/IPC. L'image récupérée par les logs bornés est inspectée
visuellement. Le build, les tests, la GUI et le MSI tournent sur des VM GitHub ;
aucune capture matérielle, lecture sonore ni installation MSI n'est exécutée
sur le PC personnel. Les sources sont publiées sur
`audit/windows-synthetic-audio-service-20261008` ; les dernières notes de
suivi seules sont locales et ne changent pas l'état applicatif validé.

Le diagnostic supplémentaire du signal import ne justifie pas de réécrire
les annotations : exclure uniquement les trames quasi nulles laisse encore
l'erreur au-dessus de 0,05. Le modèle manque une réponse courte et faible ;
une correction de couverture et sa validation sur les six scénarios restent
à établir. Les seuils, fixtures et contrôles sont conservés. Travail incomplet,
compteur final **0/2** ; les CI vertes ne sont pas comptées comme deux audits.

## Reprise après le bilan Windows

Le test indépendant du même modèle en FP32, téléchargé à la même révision
et vérifié par SHA-256, produit exactement les mêmes tours et la même erreur
import que le modèle int8. Remplacer le modèle ne résout pas l'échec. Le
diagnostic séparé des fins de clips montre des amplitudes très faibles dans
les portions encore annotées comme parole ; ces observations ne constituent
pas une référence suffisante pour modifier les annotations ou les seuils.

Deux défauts supplémentaires sont reproduits par des tests avant correction :
un layout Windows à deux canaux FC/LFE était interprété comme FL/FR ; un
client IPC Linux lent conservait son reader après retrait du writer. Le
convertisseur respecte désormais le masque de positions, et le socket est
fermé dans les deux sens avant son retrait. La stéréo FL/FR garde sa conversion
directe. Les six tests de conversion extraits, les 75 tests Linux, Clippy dans
les deux configurations et les contrôles Windows croisés réussissent. PORTING
est corrigé pour décrire les runners et les tests synthétiques actuels.
La CI native `37808502722` réussit sur le commit `799a6e8` : **83 tests
ordinaires**, **85 tests ci-audio**, Clippy dans les deux configurations,
builds release, MSI, installation/désinstallation, GUI et IPC isolé.
https://github.com/Rhadamanthe0/meeting-recorder-windows/actions/runs/37808502722.
La capture de la fenêtre de ce run est inspectée visuellement et ne présente
pas de problème de rendu identifié.

Les six scénarios sont aussi exécutés intégralement avec le binaire Linux
recompilé. Le banc obtient **5/6**, avec tous les scores inchangés : seul
`import` reste à **0,078869047619** au lieu du maximum **0,05**. Les résultats
et transcriptions sont conservés dans `/tmp/meeting-audit-bench/results-reprise.json`
et `transcripts-reprise/`. Le graphique du signal et des probabilités est dans
`/workspace/meeting-audit-artifacts/import-diagnostic.png`.

La relecture couvre aussi les interactions import/manifestes/lecteur,
édition/sauvegarde/Undo, statut IPC/fermeture des fenêtres, agents/actions,
configuration et documentation. Un débordement supposé sur des horodatages
extrêmes est écarté : le parseur rejette déjà les valeurs non représentables
en microsecondes pour le lecteur. Aucun changement n'est conservé pour cette
hypothèse. La modification distante de l'exemple PowerShell (`Read-Host`)
est intégrée sans écraser les modifications locales ; sa syntaxe est vérifiée.

La précision du modèle et les bornes de parole de la référence import restent
à départager avec une annotation indépendante. Aucune correction validée
ne permet actuellement de faire passer ce contrôle en conservant son sens.
Les fixtures, seuils et tests existants restent actifs. Le travail demeure
incomplet ; compteur **0/2**, aucun audit sans problème n'est compté.

## Reprise des références temporelles du banc

Le générateur utilisait la durée entière du WAV Piper comme durée de parole,
y compris les fins presque silencieuses. Sur le PCM exact de `import/audio.ogg`,
la plupart des derniers mots précèdent ces fins de plusieurs centaines de
millisecondes. Le diagnostic indépendant utilise Wav2Vec2 base 960h FP32, dépôt
`onnx-community/wav2vec2-base-960h-ONNX`, révision
`729c1a6730fb549c20a1c73a3d3f96f11020225e`, graphe SHA-256
`00b7cc69516c1ab63c429e63a2b543e4d42bb77441ec5b98ee935de175b00de1`,
avec sa normalisation de moyenne/variance et ses logits CTC. Il confirme par
exemple le dernier mot du premier intervenant vers 5,26 s, contre une fin de
référence de 5,60 s. Les sorties CTC ne constituent pas la nouvelle référence.

La correction emploie une règle acoustique de bord indépendante des sorties
de Nemotron : fenêtres RMS de 10 ms, plancher relatif de 60 dB et plancher
minimal d'un pas PCM 16 bits, marge conservatrice de 100 ms aux deux bords.
Elle ne retire aucun silence interne et ne modifie ni les audio, ni les textes,
ni les locuteurs, ni le score, ni le seuil de 5 %. Les références partagées
call/call-speakers/import restent identiques. Les chevauchements sont conservés
si l'autre voix occupe encore un bord. La première réponse courte de Ben reste
comptée ; ses sons faibles sont conservés et seuls les derniers blocs sous le
plancher et au-delà de la marge sont exclus. Ce diagnostic n'a nécessité aucun
périphérique audio.

Le générateur applique cette règle avant le mélange, tout en conservant les
échantillons, la durée des clips et leurs positions. Cinq nouveaux tests
couvrent les silences de bord, la voix faible, les pauses internes, la réponse
courte, le rejet d'un clip silencieux et l'absence de décalage du clip suivant.
Les 23 tests Python réussissent. Le banc complet est relancé ; compteur 0/2
jusqu'à confirmation et relectures finales.

Le banc complet après correction des références réussit les **6/6 cas**,
avec les modèles par défaut : import **0,04964176049**, limite **0,05**
inchangée. Les métriques textuelles restent identiques. Résultat réellement
exécuté : `/tmp/meeting-audit-bench/results-reference.json`. Ce succès corrige
la référence de silence, pas l'omission de la première réponse courte par le
modèle, qui reste une erreur mesurée dans le score accepté. Aucun test ou
contrôle n'a été supprimé.

La relecture trouve aussi l'appel à un exécutable `piper-tts` inexistant dans
le paquet officiel : il fournit `piper`. Le générateur utilise désormais
`sys.executable -m piper`, pour prendre le paquet de son propre environnement.
Installation isolée de piper-tts 1.8.0, aide CLI et synthèse réelle avec Joe
(ONNX/config vérifiés à la révision c10ece1aade47bb51c153c893d14e5bf8e5b7117)
réussies, exclusivement vers un WAV. Aucun périphérique de sortie ouvert.

Enfin, après déplacement d'un dossier lors d'un renommage, un échec de lecture
du transcript, d'écriture du titre ou de sauvegarde du manifeste laissait
le dossier sous le nouveau nom. La restauration ramène maintenant le dossier,
le chemin du lecteur et l'état de la page au chemin précédent. Si ce nom est
occupé, il est conservé et une erreur de restauration est signalée. Linux
utilise RENAME_NOREPLACE pour refuser également une collision concurrente.
Deux nouveaux tests vérifient les fichiers, le renommage limité à la casse et
le refus d'un ancien nom occupé. Les **77 tests Linux** et les deux Clippy
all-targets réussissent ; le build release réussit.

Parcours GTK réel sous Xvfb : l'ancien binaire reproduit le dossier déplacé
malgré l'échec ; le nouveau restaure le dossier. Le rendu montre un seul
message utile. Un renommage réussi conserve la cohérence dossier/manifeste/
titre du transcript ; une collision du manifeste et un dossier non inscriptible
provoquent tous deux la restauration attendue, sans perte du contenu.
Images et script de parcours : `/tmp/meeting-audit-title-gui/`. Compteur 0/2
après ces corrections ; nouvelle CI et deux relectures finales nécessaires.
