# Plan d'implémentation V2

La [section 13 de la spécification](product-spec.md#13-plan-dimplémentation-pour-codex) fixe le contenu normatif. Le tableau suivant relie les phases aux composants inspectés ; il ne réduit aucun périmètre. Chaque phase doit disposer de code, tests, preuves, rapport de migration/risques et commits locaux identifiables. Aucun tag ni release automatique.

| Phase | Composants principaux | Preuve de sortie |
| --- | --- | --- |
| P00 | Docs, inventaire, fixtures DB, outillage de mesure, CI | Baseline reproductible, non-régression et CI verte |
| P01 | Prototypes IDE isolés, contrats backend | Monaco, CodeMirror, Theia, base VS Code comparés avec versions/licences, LSP/DAP/terminal/extensions ; ADR justifié |
| P02 | Dashboard : tokens et composants accessibles | Deux thèmes, contrastes mesurés, focus, états et vues réelles de référence |
| P03 | Runtime/API/CLI/DB/config/paths | Deux dépôts actifs, IDs et bases isolés, migration V1 sauvegardée et restaurable |
| P04 | AppShell, Dock/Tab manager, layout durable | Arbre versionné, splits/déplacements, focus, presets et restauration exacte |
| P05 | API fichiers, éditeur retenu, documents | Édition/sauvegarde réelles, recherche, diffs et conflits de buffers sans perte |
| P06 | Processus IDE, protocoles, permissions | Rust/TS/Python, vrais LSP/DAP, terminal système, extension chargée, échecs explicites |
| P07 | Conversation, contrat mission, événements et composeurs | Mission réelle depuis chat/formulaire/graphe, données de progression vérifiables |
| P08 | Core, Runtime, DB, Git, capacité et décisions | Tâches parallèles puis dépendantes, isolation et intégration contrôlée, replan/retry bornés |
| P09 | Profils, modèles CLI, politiques et grants | Remplacement après checkpoint, conservation des tentatives et droits effectifs vérifiés |
| P10 | Leases, sessions, checkpoints, replay d'événements | Fermeture du dernier client puis crash/reprise, sans doublon ni effet distant involontaire |
| P11 | Mémoire par projet, révisions, sélection et UI | Provenance, contrôle utilisateur, historique, sélection budgétée et isolation |
| P12 | Source Control, revue, Delivery/GitHub | Objectif → intégration → PR explicitement autorisée, incertitude réconciliée |
| P13 | Notifications, paramètres, raccourcis et UX | Toutes les vues fonctionnelles, deux thèmes, états vides/erreurs et accessibilité |
| P14 | Qualification transversale et packaging | Toutes les exigences, missions réelles et quatre plateformes validées avant RC |

L'ordre de la spécification est conservé. Les choix P01/P02 précèdent la construction de l'IDE/workspace. Les interfaces de checkpoint sont anticipées en P08/P09, mais la reprise n'est qualifiée qu'en P10. Les tests P14 sont préparés dans les phases qui introduisent les capacités, puis exécutés en qualification finale.

Le statut courant de P00 est consigné dans son rapport (historical record held privately). L'évaluation locale P01 et son choix CodeMirror/Dockview/Rust sont consignés dans la livraison P01 (historical record held privately), avec les preuves et limites de packaging/CI. Les composants, prototypes, mesures et validations P02 sont consignés dans la livraison P02 (historical record held privately). P03 livre le runtime multi-projet (rapport (historical record held privately)). P04 intègre le workspace de production (rapport (historical record held privately)). P05-P14 ne sont pas commencées. Un inventaire de capacités absentes, une documentation de conception ou une compilation ne vaut pas passage de gate. Les documents techniques de la section 16 sont livrés avec leur travail effectif. Les gates distantes et plateformes restent séparées des validations locales.
## État après P06

Les indications de statut antérieures décrivent leur date de livraison. P05 a livré l'explorateur et l'éditeur réels (rapport (historical record held privately)). P06 a livré les services IDE et leur intégration de production (rapport (historical record held privately)), qualifiés localement sur Windows avec les limites documentées. Les phases P07-P14 ne sont pas commencées. Les CI exactes P00-P06 et les qualifications natives Linux/macOS restent des gates séparées ; ASF V2.0 demeure NO-GO.

## État après P07-R1 et qualification P08

Les statuts ci-dessus restent l'historique de leurs livraisons. P07 puis P07-R1 (historical record held privately) ont livré et qualifié localement la conversation et les trois entrées de missions avec de véritables modifications de code, revues et intégrations. Leurs limites de permissions, de CI et de plateformes restent documentées.

P08 (historical record held privately) étend le moteur existant avec dispatch parallèle borné, claims durables, worktrees isolés, publication sérialisée et replanifications validées. Les acceptations réelles de parallélisme, dépendances, capacité et conflit/replanification sont conservées. Une acceptation distincte de réparation sémantique a été interrompue par la limite d'utilisation du CLI, après la génération d'une révision réelle du plan. P08 demeure **PARTIAL** jusqu'à sa qualification complète ; les tests déterministes ne remplacent pas cette preuve manquante. P09-P14 ne sont pas commencées. Aucune CI distante exacte ni qualification native Linux/macOS n'est déduite des tests Windows. ASF V2.0 demeure **NO-GO**.

## P10 delivery qualification

The P10 implementation extends P08 admission and P09 safe boundaries with authenticated browser leases, explicit workspace closure, durable checkpoints and manual logical resume. The qualification outcome and executed evidence are recorded in deliveries/V2-P10.md (historical record held privately). P00-P09 historical reports remain intact. Exact-commit remote CI and native Linux/macOS qualification remain independent release gates. P11 has not begun.


## P11 project memory integration

P11 is implemented and COMPLETE LOCALLY with remote qualification pending; evidence and limits are tracked in V2-P11 (historical record held privately) and [test matrix](p11-test-matrix.md). No P12 implementation, remote push, release or publication is authorized. Local results and inherited CI/platform gaps remain separate gates.


## P13 integrated product status

Earlier status paragraphs are historical snapshots, preserved as delivery history. P08-R1 and P09?P12 were subsequently delivered on the retained ancestry; their reports remain authoritative about executed evidence and limitations. P13 consolidates Settings, actual event notifications, commands, navigation, accessibility and measured memory/Git optimizations. Current qualification is recorded in V2-P13 (historical record held privately); accumulated CI/platform/provider/AT/performance gates are in [the release qualification ledger](v2-release-qualification-ledger.md). P14 is not begun. ASF V2.0 remains **NO-GO**; no push, PR, publication, signing, deployment, tag or release is implied.
