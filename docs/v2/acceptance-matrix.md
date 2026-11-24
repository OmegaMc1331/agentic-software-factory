# Matrice d'acceptation V2.0

État initial établi en P00. La [spécification produit](product-spec.md) reste normative, y compris ses exigences détaillées non résumées dans les intitulés. Une capacité V1 réutilisable ne constitue pas une qualification V2. Tous les parcours V2 ci-dessous restent **non qualifiés** ; aucun score de complétude n'est déduit du nombre de lignes.

L'[évaluation P01](ide-evaluation.md) apporte des preuves isolées d'édition/sauvegarde, splits/diffs, LSP TypeScript/Pyright, DAP debugpy, PTY de test, plugin YAML natif, restauration et packaging Windows représentatif. Elle ne transforme aucune ligne ci-dessous en capacité de production livrée. Les parcours Rust/TypeScript/Python complets, l'extension manager, le shell de développement et les quatre plateformes restent obligatoires.

## Couverture fonctionnelle (§15.1)

P02 livre un [design system exécutable](design-system.md) et des [prototypes UX isolés](ux-prototypes.md) avec thèmes, clavier, docking, édition locale, captures et mesures. Ces preuves locales ne marquent aucune capacité de production ci-dessous comme qualifiée. F06 dispose de fondations testées ; l'intégration à toutes les vues et la qualification produit restent P13/P14.

P03 livre le runtime multi-projet et ses preuves locales (historical record held privately) : registre durable, contextes Rust distincts, routes explicites, workflows contrôlés simultanés, PTY/SSE/WebSocket isolés et interface réelle de gestion des projets. La [matrice d'isolation](project-isolation-test-matrix.md) relie les invariants aux tests. Les preuves Windows locales de F01/F13/F33 ne remplacent ni la CI du commit exact, ni les qualifications Linux/macOS, ni les missions réelles de P14. Les sessions utilisateur et leur suspension restent P07/P10.

| ID | Exigence | Phase | Situation de départ / preuve requise |
| --- | --- | --- | --- |
| F01 | Multi-projet | P03 | Implémenté et testé localement : deux bases/runtimes, workflows simultanés, IDs locaux identiques, registre et interface réels ; qualification produit multiplateforme en attente |
| F02 | Multi-session | P04/P07/P10 | P04 navigation sessions have independent persisted views/layouts; conversation ownership and suspension remain P07/P10 |
| F03 | Onglets déplaçables | P04 | Local production tests: stable IDs, drag/command reorder, cross-group movement, reopening, navigation-only close; final product/platform qualification pending |
| F04 | Vues fractionnées | P04 | Local Dockview production splits, resizing, keyboard commands and project-owned views; final product qualification pending |
| F05 | Dispositions restaurées | P04/P10 | P04 versioned presentation tree, tabs/drafts/theme and revision conflict recovery; execution recovery remains P10 |
| F06 | Thèmes clair/sombre | P02/P13 | Deux thèmes complets à vérifier, contrastes mesurés et reduced motion |
| F07 | Orchestrateur conversationnel | P07 | Absent ; conversation persistante et réponses liées aux preuves |
| F08 | Trois méthodes de création | P07 | Formulaire V1 partiel ; chat/formulaire/plan visuel vers le même contrat réel |
| F09 | Progression compacte | P07 | Comptes de tâches V1 réutilisables ; aucune estimation arbitraire de pourcentage |
| F10 | Panneau Agents | P07/P09 | Configuration/supervision V1 ; panneau indépendant, capacité et tâches réelles |
| F11 | Agents personnalisés | P09 | Exécutables/rôles V1 ; profils validés avec instructions/outils/droits |
| F12 | Modèle par agent | P09 | Arguments génériques V1 ; capacités effectives des CLI, absence exposée clairement |
| F13 | Permissions et plafonds par projet | P03/P09 | P03 préserve les politiques V1 par contexte et ajoute des plafonds globaux appliqués côté Rust ; isolation configuration/MCP testée ; profils/politiques V2 versionnés et audit P09 à terminer |
| F14 | Ordonnancement dynamique | P08 | Boucle séquentielle V1 ; dépendances/capacité/conflits/permissions arbitrés par Rust |
| F15 | Tâches parallèles isolées | P08 | Worktrees V1 ; preuve d'exécution simultanée et intégration contrôlée |
| F16 | Replanification | P07/P08 | Modèles/révisions DB V1 ; décisions et graphe réel, historique immuable |
| F17 | Réparation et retry | P08 | Retry borné V1 ; observer/diagnostiquer/corriger/vérifier avec classes d'échec distinctes |
| F18 | Remplacement par checkpoint | P09/P10 | Absent ; transfert, arrêt d'attribution, révocation et reprise fidèle |
| F19 | Suspension à la fermeture | P10 | Absent ; dernier client, heartbeat/lease/grâce, plusieurs fenêtres |
| F20 | Reprise après incident | P10 | Réconciliation V1 ; checkpoints logiques, validations Git/droits et pas de rejeu externe |
| F21 | Mémoire persistante structurée | P11 | Index de contexte seulement ; catégories, provenance, révisions, retrait et sélection |
| F22 | Explorateur | P05 | Absent ; CRUD, worktrees, binaires et périmètres/links contrôlés |
| F23 | Éditeur | P05/P06 | Absent ; sauvegarde/undo/recherche/formatage/diff/conflits et plusieurs documents |
| F24 | LSP | P06 | Absent ; diagnostics, completion, hover, définition/références et opérations disponibles |
| F25 | DAP | P06 | Absent ; lancement/attachement supporté, breakpoints/steps/pile/variables/console |
| F26 | Terminal | P06 | PTY agent V1 ; shell de développement autorisé et réellement connecté |
| F27 | Extensions | P01/P06 | MCP V1 distinct ; découverte/import, installation, cycle de vie, droits et erreurs |
| F28 | Source Control | P12 | Moteur Git V1 ; stage/commit/branches/historique/conflits et provenance des modifications |
| F29 | Revue | P12 | Preuves/revues V1 ; vue complète et parcours simplifié, aucun succès inféré d'une seule approbation |
| F30 | Livraison configurable | P09/P12 | Delivery V1 ; politiques explicites, preview, PR et réconciliation des incertitudes |
| F31 | Notifications | P13 | Centre absent ; catégories §4.11 et notifications système optionnelles |
| F32 | Historique | P07–P13 | Historique d'exécution V1 ; conversations, événements et décisions liés au projet/session |
| F33 | Compatibilité projets V1 | P00/P03/P14 | P03 ouvre la fixture V1 figée, conserve historique/config/worktrees et CLI ; backup SQLite vérifié et rollback transactionnel testés ; qualification finale et plateformes en attente |

## Missions réelles (§15.2)

Toutes : **non testées en P00**. Chaque résultat futur doit identifier dépôt, commits initiaux/finaux, agents réels, permissions, événements, tests, artefacts et limitations. Une fixture ou un faux exécutable ne remplit aucune de ces lignes.

| ID | Scénario obligatoire | Préparation / qualification |
| --- | --- | --- |
| M01 | Correction simple Rust | P07/P14 |
| M02 | Fonctionnalité TypeScript | P07/P14 |
| M03 | Modification Python | P07/P14 |
| M04 | Deux tâches indépendantes simultanées | P08/P14 |
| M05 | Tâches parallèles et dépendantes | P08/P14 |
| M06 | Compilation échouée, diagnostic, réparation | P08/P14 |
| M07 | Test échoué puis replanification | P08/P14 |
| M08 | Remplacement après checkpoint | P09/P10/P14 |
| M09 | Conflit d'intégration Git | P08/P12/P14 |
| M10 | Fermeture navigateur puis reprise | P10/P14 |
| M11 | Arrêt brutal runtime puis réconciliation | P10/P14 |
| M12 | Livraison GitHub autorisée et résultat distant incertain contrôlé | P12/P14 |

## IDE (§15.3)

Toutes : **non testées / capacités absentes en V1**, sauf le PTY d'agent distinct. P01 doit produire des prototypes et mesures avant le choix ; P05/P06/P14 doivent démontrer l'intégration finale.

| ID | Preuve obligatoire |
| --- | --- |
| I01 | Édition et sauvegarde réelles |
| I02 | Recherche globale réelle |
| I03 | Diagnostics LSP réels |
| I04 | Navigation vers définition réelle |
| I05 | Débogage DAP réel |
| I06 | Terminal connecté à un shell autorisé |
| I07 | Extension réellement chargée |
| I08 | Conflit édition manuelle / modification d'agent résolu sans perte |
| I09 | Restauration fichiers, buffers et panneaux |

La qualification couvre au minimum Rust, TypeScript et Python. Une fonctionnalité d'adaptateur absent est indisponible ; elle ne doit pas être simulée.

## Sécurité (§15.4)

| ID | Invariant | Preuve finale attendue |
| --- | --- | --- |
| S01 | Aucun accès fichier API hors périmètre | Traversées, liens/junctions, alias, binaires, courses/conflits et écritures contrôlées |
| S02 | Aucune opération GitHub non autorisée | Refus par défaut, politique/confirmation et droits revérifiés côté Rust |
| S03 | Pas de secrets dans les événements | Redaction avant persistence/replay/SSE/WS et tests de fragments |
| S04 | Pas de répétition dangereuse d'une publication incertaine | Identité d'opération, single-flight, crash et réconciliation réelle/contrôlée |
| S05 | Permissions côté Rust et worktrees coordonnés | Tentatives, refus, ownership et intégration liés aux bons SHA |
| S06 | Modèle de confiance explicite pour CLI/LSP/DAP/extensions | Permissions de processus, origine/auth/CSRF/WS, pas d'assimilation à un sandbox OS |

Les suites V1 et les contrôles ponctuels de baseline sont réutilisables, mais ne couvrent pas les nouvelles surfaces avant leur implémentation.

## Vues, UX et performance

UI-01 à UI-18 restent à qualifier selon la section 4. L'inventaire de leurs interactions, pas seulement leur présence visuelle, fait partie des gates P02/P04–P13. Les parcours A–E (§2), les quatre presets (§5.8), les tailles/zoom, le clavier et WCAG 2.2 AA applicable (§5.7) sont obligatoires en P14.

La baseline mesure uniquement des opérations V1 réellement disponibles et indique ce qui n'est pas mesuré. La section 15.5 exige ensuite startup, ouverture, restauration, chargement fichier, grand dépôt, recherche, réaction UI, mémoire LSP/extensions, terminal, exécution multi-agent et reprise. Les seuils chiffrés seront justifiés à partir de ces mesures et des prototypes P01 ; aucun seuil n'est validé en P00.

## Plateformes et distribution (§15.6)

| Cible | Preuve V1 disponible | Qualification V2 |
| --- | --- | --- |
| Windows x86_64 | CI du commit de référence et vérifications locales P00 consignées | Non testée |
| Linux x86_64 | CI de référence, embedded smoke et installateurs | Non testée |
| macOS Apple Silicon | CI de référence `macos-latest` | Non testée |
| macOS Intel | Cible du workflow de release ; aucune nouvelle preuve native P00 | Non testée |

P14 doit vérifier pour chaque cible : installateur/lancement/auth navigateur/ressources embarquées/terminal/adaptateurs/migration V1/intégrité/checksums/réinstallation/mise à jour. Une cross-compilation ne vaut pas un essai natif. Les tests d'endurance, audits de dépendances/licences et anomalies P0/P1/P2 sont suivis avec le candidat final.

**Décision : NO-GO V2.0** tant que les preuves obligatoires ne sont pas toutes satisfaites et que la CI du candidat exact n'est pas verte.
