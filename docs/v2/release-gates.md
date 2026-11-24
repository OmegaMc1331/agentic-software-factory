# Gates V2.0

**Décision actuelle : NO-GO V2.0.** La baseline V1 ne qualifie ni l'IDE ni les parcours V2. La [spécification, section 15](product-spec.md#15-critères-dacceptation-de-la-release-v20), est normative ; la [matrice d'acceptation](acceptance-matrix.md) suit les preuves.

| Gate | Conditions de passage |
| --- | --- |
| Fonctionnel | Toutes les capacités §15.1 utilisables, sans compteurs simulés ni substituts décoratifs |
| V1 | CLI, endpoints, historiques, profils, politiques, worktrees et garde-fous conservés ou remplacés explicitement |
| Réel | Douze scénarios de missions §15.2 dans de vrais dépôts avec de vrais agents ; fixtures déterministes séparées |
| IDE | Sauvegarde/recherche/diagnostics/définition/DAP/terminal/extension réels ; conflit de buffer et restauration vérifiés |
| Sécurité | Scopes fichiers/permissions Rust, secrets, origine/auth/CSRF/WS, isolation Git et réconciliation externe vérifiés |
| Résilience | Dernier client, plusieurs fenêtres, délai de grâce, crash runtime, checkpoints et reprise sans duplication |
| Performance | Machine et protocoles fixés, mesures brutes, médiane/p95 et seuils justifiés après baseline/prototypes ; aucun seuil inventé |
| Qualité | Aucun P0/P1 ouvert ; P2 corrigés ou explicitement évalués ; tous les jobs CI obligatoires verts pour le commit candidat |
| Migration | Sauvegarde cohérente, intégrité, ouverture d'un vrai projet V1, consultation/reprise et retour sur sauvegarde démontrés |
| Distribution | Windows x86_64, Linux x86_64, macOS Apple Silicon et Intel ; installateurs, binaires embarqués, checksum, réinstallation et mise à jour V1 |
| UX | Tous les écrans, thèmes clair/sombre, focus, raccourcis, zoom, lecteur d'écran applicable, contrastes et reduced motion vérifiés |

Les jobs existants de CI restent obligatoires : Rust et checksums sur Windows/Linux/macOS, frontend, embedded-dashboard et installateurs. Le job macOS habituel cible Apple Silicon ; la compilation Intel du workflow de release ne remplace pas son test natif. Les modifications P00 ajoutent la vérification du socle V1 aux jobs checksums sans retirer les gates existantes.

Un résultat est étiqueté `mesuré`, `dérivé`, `synthétique`, `non testé`, `indisponible` ou `refusé/échoué` selon son origine. Une assertion d'agent, un mock ou une revue favorable ne remplace pas une preuve d'exécution. Une preuve doit identifier le projet, les commits, le scénario, le résultat et ses limites.

Un GO exige toutes les lignes obligatoires satisfaites pour le même candidat, avec les résultats de plateforme et les défauts consultables. Toute absence maintient NO-GO. La décision de publier reste distincte de cette qualification ; aucune publication n'est déclenchée par ce dossier.
