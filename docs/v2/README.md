# ASF V2 — dossier de référence

La [spécification produit](product-spec.md) fournie par l'utilisateur est la référence normative. Son statut décrit la conception initiale ; l'avancement d'implémentation est suivi séparément dans les livraisons et les matrices ci-dessous.

| Document | Usage |
| --- | --- |
| [Baseline](baseline.md) | Référence, commandes reproductibles, mesures et limites de qualification |
| [Architecture actuelle](current-architecture.md) | Composants V1, stockage, tous les endpoints et modèles SQLite |
| [Évaluation IDE P01](ide-evaluation.md) | Quatre candidats, prototypes réels, mesures, sécurité, licences et choix |
| [ADR du socle IDE](adr-ide-foundation.md) | CodeMirror, Dockview et autorité Rust ; conséquences et prérequis |
| [Contrats IDE cibles](architecture-target.md) | Processus, documents, protocoles, extensions, persistance et frontières |
| [Prototypes reproductibles](../../experiments/v2-ide-evaluation/README.md) | Commandes, fixtures isolées, packaging local et pipeline proposé |
| [Design system P02](design-system.md) | Tokens, thèmes, typographie, interactions, accessibilité et limites |
| [Inventaire des composants](component-inventory.md) | Contrats et états des 58 composants réutilisables |
| [Prototypes UX](ux-prototypes.md) | Exécution locale, scénarios, interactions réelles et données fixtures |
| [Workspace visuel](workspace-visual-spec.md) | Dimensions, presets, focus, redimensionnement et restauration |
| [Multi-project P03](multi-project-architecture.md) | Durable registry, Rust contexts, locks, quotas and isolation |
| [API contracts](api-contracts.md) | Project management, explicit routing and immutable V1 default |
| [V1 migration](migration-v1-v2.md) | Preservation, verified SQLite backups and recovery |
| [Isolation matrix](project-isolation-test-matrix.md) | Invariants I1-I12 and scenarios MP-01-MP-15 |
| [Workspace P04](workspace-architecture.md) | Production shell, scoped views and lifecycle |
| [Navigation](navigation-contracts.md) | Projects, navigation sessions, view operations and keyboard |
| [Layouts](layout-persistence.md) | Versioned Rust storage, restoration and multi-window recovery |
| [Compatibilité](compatibility-matrix.md) | Contrats V1 à préserver et risques de migration |
| [Plan d'implémentation](implementation-plan.md) | Phases, dépendances et règles de passage |
| [Acceptation](acceptance-matrix.md) | Exigences de release et preuves encore nécessaires |
| [Gates de release](release-gates.md) | Décision NO-GO, conditions de GO et plateformes |
| [Inventaire gelé](../../scripts/fixtures/v1-inventory.json) | Routes, hashes des migrations, schéma, colonnes, clés étrangères et index |


La [qualification des versions](release/README.md) d?crit les contr?les et les ?l?ments reproductibles. Les preuves historiques restent dans les archives priv?es du projet. ASF V2.0 reste NO-GO.
