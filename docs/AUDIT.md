# Audit correctif du dépôt — 8 octobre 2026

Compteur provisoire : **0/2**. Les dernières corrections nécessitent encore
les deux relectures complètes sur le même état, avec validations réussies.

## Périmètre et préparation

Le dépôt était propre au départ. Aucun AGENTS.md ou CLAUDE.md applicable n'a
été trouvé. README, PORTING, documentation des actions, du banc et des démos,
Cargo, workflows et scripts de packaging définissent les comportements et
contrôles attendus. Les modifications utilisateur et les merges présents sur
la branche de publication sont conservés. Aucun merge vers master, tag,
déploiement ni publication de release n'a été effectué.

La revue porte sur le code applicatif maintenu, les tests, les exemples,
les scripts, les configurations, la documentation et les correctifs locaux
aux dépendances vendoriées. L'amont C/C++ figé n'a pas reçu une revue ligne
par ligne exhaustive.

## Corrections appliquées

| Zone | Défaut et correction |
| --- | --- |
| Audio et données | Erreurs d'écriture perdues à l'arrêt et erreurs de synchronisation d'un ancien enregistrement : lecture sous le même verrou et identification de l'enregistrement. Fermeture du fichier temporaire avant nettoyage sous Windows. Protection des manifestes lors d'un renommage limité à la casse ou d'un échec de suppression. |
| Sécurité | WASAPI 0.24 concerné par RUSTSEC-2026-0332 remplacé par 0.25. Trames IPC tronquées refusées ; titres trop longs refusés par la CLI. Permissions du pipe Windows restreintes au propriétaire et à LocalSystem. Messages non interprétés comme du markup. |
| Interactions UI | Vérification de l'état après les dialogues d'import ; sauvegarde avant modification de l'état ; rejet des résultats asynchrones d'une autre réunion ou révision ; restauration des textes lors d'échecs de sauvegarde. Actions de paragraphe visibles au clavier et colonne d'actions défilable sur les écrans bas. |
| Ressources et délais | Références faibles des callbacks du lecteur pour libérer les réunions fermées. Déconnexion des clients IPC Windows qui ne lisent plus, pour libérer les workers et handles bloqués. Job Objects pour les outils audio Windows, incluant lecture, import, export, décodage, sondes et formes d’onde. Sondes d'agents limitées en temps et en taille ; arrêt des descendants avant d'attendre l'envoi du texte. |
| Entrées et configuration | Horodatages et couleurs invalides sans panic ni débordement ; caractères de contrôle filtrés des noms. Lecture des paramètres racine avec guillemets, chemins Windows et caractères # préservés. Chemins des actions rendus absolus avant changement de répertoire. |
| Outillage et documentation | Exemple compilable avec all-targets et corrections des diagnostics des contrôles existants. Échecs WiX bloquants, version MSI issue de Cargo, schémas GTK limités au processus. Suivi Cargo de GGML_NATIVE et des options CMake transmises, pour éviter un cache de compilation obsolète. Banc utilisant le vrai nom du binaire et refusant erreurs ou mesures absentes. Installation source et limites documentées corrigées. Notes Obsidian : échappement des noms en YAML et écriture UTF-8 explicite. |

- Renommage de réunion : restauration du dossier si la lecture ou l'écriture
  du transcript ou du manifeste échoue ; collision refusée sans écrasement.
- Transcription et chapitres : sauvegarde du texte avant le manifeste,
  restauration des anciens octets en cas d'échec et validation de l'état
  mémoire après succès. Il s'agit d'une gestion des erreurs d'I/O, pas d'une
  transaction résistante à un crash entre deux fichiers.
- Whisper : propriété explicite des callbacks pendant l'appel synchrone,
  puis libération ; validation des langues avant tout travail ; stockage
  CString partagé et libéré dans une copie du même whisper-rs 0.16.0.
  Provenance, licence et correctif documentés dans third-party/whisper-rs.
- Banc : invocation de Piper par son module Python officiel ; limites de
  parole corrigées par une règle RMS fixe (60 dB, pas de 10 ms, marge 100 ms)
  retirant seulement le silence de bord des clips. Audio, paroles, locuteurs,
  chevauchements, modèles, métriques et seuils conservés. La réponse très
  faible de Ben reste comptée comme manquante. Tests de la règle ajoutés.
- Mise en page : un nom de locuteur long imposait une largeur de 1034 pixels
  et réduisait le texte à quelques caractères par ligne. Colonne bornée et
  nom ellipsé avec infobulle complète ; rendu réel vérifié à 820 × 560,
  sans modification du transcript ni du manifeste.
- Accessibilité : noms de locuteurs rapprochés de la couleur du texte tout
  en conservant des teintes distinctes, dans les styles standard et Omarchy.
  Le rendu réel des six couleurs en clair/sombre, y compris la ligne
  sélectionnée, donne des contrastes compris entre 5,82 et 11,67.
- CI Windows : toolchain GNU vérifiée avant/après cache, préparation sûre
  de MSYS2 et PowerShell pour le compte de service, outils vérifiés par hash,
  fermeture des processus ciblée et tests GUI/MSI sur VM GitHub.

## Validations réalisées avant les deux passes finales

- Linux : fmt, check all-targets, Clippy all-targets bloquant normal et
  ci-audio, build release ; 80 tests applicatifs et un test d'intégration
  mesurant les allocations de langue réussis dans les deux configurations.
  Les avertissements des bindings whisper-rs-sys ne sont pas masqués.
- Python : 23 tests du banc réussis. Génération réelle Piper vérifiée dans
  un environnement isolé, sans sortie audio matérielle.
- Banc complet : six cas réussis à plusieurs reprises ; erreur de locuteur
  import 0,0496417604912999 pour un maximum inchangé de 0,05.
- RustSec : 278 dépendances, aucune vulnérabilité ni avertissement signalé
  par la base actualisée (1295 avis). Recherche de secrets sans résultat.
- Syntaxe : Python, YAML, JSON, XML, shell et 53 blocs/scripts PowerShell
  analysés avec PowerShell 7.6.6.
- GTK Linux réel sous Xvfb : édition, suppression/Undo, renommage réussi et
  échecs de sauvegarde, retour à une nouvelle réunion, rendu à 820 × 560 ;
  quatre rendus clair/sombre standard et Omarchy inspectés.
- Windows VM, ce2ad746, run 37819848984 : build, tests, analyse statique,
  interface synthétique et installation/désinstallation MSI réussis.
  La dernière modification CSS doit encore recevoir la même validation.
- CLI et exemples : entrées invalides refusées avec un seul message utile,
  exemple Obsidian testé avec Unicode, guillemets et antislashs.

Les résultats, captures et scripts de diagnostic sont conservés dans
/tmp/meeting-audit-* et les logs du workflow Windows. Une exécution du banc
a été interrompue par le redémarrage de l'environnement ; elle n'est pas
comptée comme réussie. Le premier téléchargement de modèle a échoué avec
UnknownIssuer : les modèles ont ensuite été téléchargés avec les certificats
système approuvés et leurs hashes vérifiés ; TLS n'a pas été désactivé.

## Limites et suivi

Aucun défaut confirmé restant après les corrections ci-dessus ; compteur
encore à 0 tant que les deux audits complets ne sont pas terminés.

Les tests ne capturent ni ne jouent de son sur le PC personnel : ci-audio
utilise des sons en mémoire et un lecteur silencieux, un pipe et des données
isolés. Les tests GUI et l'installation MSI s'exécutent sur VM GitHub.
Les pilotes WASAPI, le hotplug matériel, l'intégration complète Quickshell/
Omarchy, Vulkan, l'accès IPC réel entre deux comptes et la mise à jour d'une
installation MSI existante ne sont pas exercés. Ces limites ne constituent
pas une garantie d'absence absolue de défauts.
