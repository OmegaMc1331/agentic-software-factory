# Fixture ASF V1 1.0.5

`asf-v1-1.0.5.sql` est un export SQL autonome, gelé depuis les 18 migrations de `580eeaa29117edc7c2a6255341483dc798890f11`. Les données sont entièrement **synthétiques**. Les commits, tests, sorties, approbations, durées et références GitHub/MCP qu'elle contient ne sont pas des preuves de travaux réellement exécutés. Aucun token ni secret utilisateur n'est inclus.

Chaque table applicative et `schema_migrations` est peuplée : workflows terminé/en attente, tâches et dépendances, tentatives, sessions automatisées/interactives, audit de politique et capture, artefact, plan, recette immuable, décision de routing, intégration, Issue, livraison incertaine avec identité d'opération, outils, grant révoqué, invocation et lease inactive. Les chemins de worktrees sont des noms logiques ; aucun worktree réel n'est livré par cette fixture.

La restauration d'un dump SQLite nécessite de désactiver les clés étrangères **durant l'import uniquement**, car l'ordre alphabétique du dump peut référencer une table créée plus tard. Les tests réactivent les contraintes et exigent `integrity_check=ok` et zéro résultat à `foreign_key_check`. `FactoryDb::open` conserve son activation de `foreign_keys=ON`.

Les tests Rust restaurent dans un répertoire temporaire, ouvrent deux fois via le véritable `FactoryDb`, comparent chaque colonne/ligne historique, exercent les lectures typées et vérifient la conservation de la livraison `interrupted/recovery_required`. Le script Python vérifie son hash canonique (texte UTF-8 à fins de ligne LF) et son schéma. Les migrations futures doivent être testées contre cette fixture, jamais en régénérant silencieusement son schéma depuis leur nouveau code.

Cette fixture complète les anciennes fixtures internes V1/V5. Elle ne remplace pas les futurs tests d'ouverture d'un vrai projet V1, de sauvegarde WAL, de déplacement des chemins absolus et de conservation des worktrees.
