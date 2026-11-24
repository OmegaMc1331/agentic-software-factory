# V2-P00 — compatibilité V1 et risques de migration

Référence : `580eeaa29117edc7c2a6255341483dc798890f11`. « Existant » signifie constaté dans le code et ses contrats ; les validations réellement exécutées sont consignées dans P00 (historical record held privately). Les scénarios à faux agents sont des tests déterministes, pas une qualification par agents réels.

## Contrats à préserver

| Contrat V1 | Source / tests de référence | Traitement V2 |
| --- | --- | --- |
| `factory init/start/status`, `mcp call`, `dev` | `factory-cli`, `tests/runtime_lock.rs`, `tests/mcp_cli.rs` | Maintien ou migration explicite ; pas de recréation obligatoire de projet |
| API sémantique, JSON camelCase, liste `/runs` non paginée et pagination optionnelle | `factory-api/tests/api.rs`, inventaire gelé | Couche V1 stable pendant le routage multi-projet P03 |
| Auth, Host/Origin, CSRF, SSE/WS authentifiés | `factory-api/tests/security.rs`, frontend `api-auth.test.ts` | Étendre l'identité de projet sans retirer ces contrôles |
| SQLite, 18 migrations et tous les historiques | `factory-db` et nouvelle fixture `tests/v1_compatibility.rs` | Ajouter des migrations après 18 ; conserver tables/identifiants et leurs références |
| Rôles, assignments, normalisation legacy avec backup | `factory-core/tests/agent_config.rs`, `role_routing.rs`, `factory-api/tests/roles_api.rs` | Réutiliser pour les profils, sans duplication de logique |
| Profils d'invocation CLI, résolution Windows, transport sans interpolation de mission | `factory-agent`, `tests/process_io.rs`, `factory-core/tests/agent_config.rs` | Rendre explicites les réglages de modèle réellement pris en charge |
| Politique effective deny-wins, protection `.git/.factory`, budget de retry | `factory-policy`, `factory-core/tests/policy_enforcement.rs` | Ajouter les opérations IDE et l'audit de politique en P09 |
| Redaction incrémentale, UTF-8, sortie bornée | `factory-agent/tests/process_io.rs`, `factory-core/tests/streaming_redaction.rs` | Préserver avant persistence, SSE et console |
| Worktrees, revue structurée, commits et branche d'intégration | `factory-core/tests/workflow_runtime.rs`, `e2e.rs`, `factory-git` | Scheduler parallèle P08 et Source Control P12 gardent l'intégration contrôlée |
| DAG et révisions sans réécriture des tâches exécutées | `factory-types/src/plans.rs`, `factory-db` | Exposer le vrai modèle aux trois composeurs P07/P08 |
| Réconciliation au démarrage et verrou canonique de projet | `factory-runtime/tests/project_lock.rs`, `factory-cli/tests/runtime_lock.rs` | Un verrou par contexte projet, ownership des opérations maintenu |
| Issues non fiables, confirmation de PR, livraison incertaine et CAS d'opération | `factory-core/tests/github_delivery.rs`, `factory-api/tests/github_api.rs` | P10/P12 ne rejouent jamais une publication inconnue |
| Contexte borné, chemins/liens validés, gitignore, recherche | `factory-context/tests/search_boundaries.rs`, `gitignore_differential.rs` | Conserver les garanties V01–V03 ; durcir pour les écritures P05 |
| Évaluations issues des tentatives, routing et tailles d'échantillon | `factory-eval/tests/evaluation.rs`, `scalability.rs`, `factory-core/tests/routing.rs` | Aucune métrique synthétique de progression ; résultats rattachés au bon état Git |
| Recettes TOML, snapshots immuables des runs | `factory-core/tests/recipes.rs`, `factory-api/tests/recipes_api.rs` | Réutiliser comme préférences de mission |
| MCP : grants hashés/expirés/révoqués, politiques et audit | `factory-mcp/tests/stdio_mcp.rs`, `http_mcp.rs`, `factory-api/tests/mcp_api.rs` | Aucun remplacement par des appels bruts d'extension |
| État visuel et sauvegardes atomiques avec concurrence de config | `factory-api/tests/config_concurrency.rs`, tests frontend de graphe et conflits | Préserver les métadonnées V1 ; les convertir explicitement si nécessaire |
| PTY/ConPTY agent, resize, bornes d'abonnés | `factory-runtime`, `factory-api/tests/api.rs`, `InteractiveTerminal.test.tsx` | Terminal de développement distinct en P06 ; pas de changement implicite de capacité |
| Dashboard embarqué, installateurs, archives et checksums | CI embedded/installer, `scripts/release-checksums.test.mjs` | Tous les jobs restent obligatoires ; tests natifs supplémentaires en P14 |

## Risques et traitement attendu

| Risque | Conséquence | Gate / mitigation obligatoire |
| --- | --- | --- |
| Changer une racine partagée pendant une requête | Lecture/écriture dans le mauvais projet | P03 : registre d'instances isolées et identités stables, deux projets simultanés |
| Collision d'IDs SQLite entre projets | Confusion de run/tâche/session | Clés de routage composites projet + ID local, aucune recherche globale par ID seul |
| Confondre sessions d'agents et sessions utilisateur | Suspension ou reprise du mauvais travail | P07/P10 : relation durable distincte sans renommer les invocations |
| Copier une DB ouverte sans son WAL | Sauvegarde incomplète | P03 : backup SQLite cohérent, intégrité et restauration testées avant migration |
| Chemins de worktrees historiques absolus | Worktrees perdus au déplacement du dépôt | Inventorier, vérifier le Git dir commun, proposer la récupération ; ne pas supprimer les données |
| Modifier migrations ou parseurs d'anciens statuts | Corruption sémantique silencieuse | Prefixe 1–18 gelé ; fixture SQL autonome + lectures typées après deux ouvertures |
| Nouveaux statuts ignorés par V1 | Fallback actuel vers `planned/pending` possible | Migration et compatibilité explicites des lecteurs avant introduction en P10 |
| Branches parallèles basées sur un état périmé | Revue/tests du mauvais contenu, conflits | P08 : worktree par propriétaire, intégration sérialisée, SHA source/revue/integration vérifiés |
| Reconnexion SSE considérée complète | Perte d'événements pendant la déconnexion | P07/P10 : journal durable, curseurs et déduplication |
| Timeout interprété comme échec sans effet distant | Double push/PR | Conserver `interrupted`, `operation_id`, réconciliation et single-flight |
| Écriture IDE, symlink/junction ou collision de buffer | Fuite ou perte de code | P05 : scopes Rust, ouverture contrôlée, version attendue, conflit explicite, récupération des buffers |
| LSP/DAP/extensions lancés sans droits explicites | Nouvelle exécution avec les droits utilisateur | P01/P06/P09 : contrats de confiance et capacités, contrôles avant lancement, erreurs d'adaptateur visibles |
| Nouveau stockage global invisible | Données de projets mélangées ou difficiles à exporter | P03 : emplacement documenté, schéma versionné, isolation et export |
| Compter une fixture comme mission réelle | Faux GO | P14 : preuves réelles séparées des tests déterministes ; matrice bloquante |
| Charge de la machine de référence | Échecs d'allocation et résultats non comparables | Logs des refus conservés, concurrence bornée, mesures backend étiquetées avec leur protocole |

## Limites de cette livraison

P00 n'introduit aucune migration applicative et n'ouvre pas les données utilisateur pour les modifier. La fixture autonome est synthétique, produite depuis les migrations du commit de référence, puis gelée : elle couvre chaque table mais ne remplace pas une migration d'un vrai projet V1 avec ses worktrees. Cette validation reste obligatoire en P03/P14.

Le contrôle statique CI détecte les suppressions de méthodes/routes V1, les modifications de migrations 1–18 et les changements de fixture. Il permet les ajouts V2. Il ne prouve pas à lui seul la compatibilité des corps HTTP : les tests d'API existants et les futurs tests de migration restent nécessaires. L'extracteur cible les déclarations Axum directes actuelles ; un changement de construction du routeur demandera une adaptation explicite de l'outil.
