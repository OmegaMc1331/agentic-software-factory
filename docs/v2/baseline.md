# V2-P00 — baseline reproductible

La référence est `580eeaa29117edc7c2a6255341483dc798890f11` (ASF `1.0.5`). Le HEAD initial correspondait exactement et l'arbre Git était propre. La [spécification produit](product-spec.md) reste la référence V2 ; cette baseline décrit la V1 et l'outillage ajouté pour la préserver.

## Périmètre et sources

Documentation lue : README, CONTRIBUTING, développement, architecture, sécurité, API et rapports de stabilisation N01–N08, F01–F04, V01–V03. Le code inspecté couvre le routeur/API, l'authentification et les transports, Runtime/verrou, Core/config/exécution, Types, les migrations DB, le dashboard et les workflows CI/release. Les détails sont reliés aux composants dans [current-architecture.md](current-architecture.md) et aux tests dans [compatibility-matrix.md](compatibility-matrix.md).

La référence comporte 14 crates Rust, un dashboard React/TypeScript/Vite, 46 chemins d'API fonctionnels plus un fallback 404, 18 migrations, 16 tables et 21 index explicites. [L'inventaire gelé](../../scripts/fixtures/v1-inventory.json) conserve méthodes/routes, colonnes, défauts, clés étrangères, DDL et hashes de migrations. Les index automatiques SQLite ne sont pas comptés dans les 21 index explicites.

Aucune migration, transition, permission, invocation CLI, opération Git/GitHub ni interface de production n'est changée par P00. Le code ajouté concerne les fixtures/tests et les commandes d'inventaire/mesure. Les contrôles CI existants sont conservés et enrichis de ces tests d'outillage.

## Machine et environnement

Vérifications locales des 26–27 octobre 2026, Windows x86_64 :

| Élément | Valeur constatée |
| --- | --- |
| OS | Windows 11 Famille, `10.0.26200`, build `26200` |
| CPU | Intel Core i5-12500H |
| RAM physique | `16 802 754 560` octets, environ 15,65 GiB |
| Rust | `rustc 1.99.1 (58cf8f9c3 2026-10-13)` |
| Cargo | `1.99.1 (5f94df478 2026-08-27)` |
| Node / npm | `v24.22.1` / `12.0.2` ; CI existante sur Node 22 |
| Python local / nouvelle CI outillage | `3.14.5` / `3.12` |

La machine partage sa mémoire avec d'autres processus. Lors des premiers échecs : `CommittedBytes=32 848 592 896`, `CommitLimit=33 982 623 744`, `AvailableMBytes=1720`. Vitest a perdu sept workers par refus d'allocation ; une compilation isolée a ensuite rencontré `os error 1455` et une autre `rustc-LLVM ERROR: out of memory`. Ce ne sont pas des tests réussis. Le premier workspace Rust a aussi observé l'échec `role_aware_artifacts_and_stages_are_exposed` (run Failed alors que la tentative était en revue) ; sa cause n'est pas déduite automatiquement du manque de mémoire.

Les validations bornées réduisent la concurrence, sans supprimer ni désactiver de test et sans modifier de plafond/politique applicative. Les journaux initiaux restent dans `.factory/v2/p00/`. Le rapport de livraison (historical record held privately) précise les résultats finaux et les incertitudes restantes.

## Reproduire les contrats de référence

Prérequis : Rust stable avec rustfmt/Clippy, Git, Node et Python 3.12+. Les fixtures n'utilisent aucun agent authentifié. Depuis la racine :

```text
python -B scripts/v2_baseline.py inventory --check
python -B -m unittest discover -s scripts -p test_v2_baseline.py
cargo test -p factory-db --test v1_compatibility -j 1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -j 1 -- -D warnings
cargo test --workspace -j 1
node --test scripts/release-checksums.test.mjs
```

Depuis `apps/dashboard` :

```text
npm ci
npm run format:check
npm run lint
npm run typecheck
npm test
npm run build
```

Sur une machine à ressources limitées, tous les tests sont aussi exécutables en série : `cargo test --workspace -j 1 -- --test-threads=1` et `npm test -- --maxWorkers=1`. Le rapport doit identifier le mode réellement passé. Ces options ne prouvent pas la concurrence V2 et ne remplacent pas la CI standard.

`inventory --check` permet l'ajout de routes/migrations V2, mais refuse le retrait d'une méthode/route V1, la réécriture d'une migration historique ou un changement de la fixture gelée. Son hash utilise le texte SQL UTF-8 à fins de ligne LF, pour rester identique sur les checkouts Windows et Unix. Les hashes des binaires restent calculés sur leurs octets exacts. Pour inspecter l'état courant sans écraser la référence :

```text
python -B scripts/v2_baseline.py inventory --output .factory/v2/p00/current-inventory.json
```

La [fixture SQL](../../crates/factory-db/tests/fixtures/README.md) est autonome et synthétique, avec chaque table peuplée. Les tests restaurent une nouvelle DB, contrôlent intégrité et clés étrangères, l'ouvrent deux fois via le véritable `FactoryDb` et comparent chaque ligne/colonne historique. Ils lisent aussi les types métier et vérifient que la livraison incertaine conserve son identité et l'obligation de réconciliation. Les anciens tests de migration V1/V5 restent présents.

## Reproduire les mesures backend V1

Construire le binaire depuis le workspace, puis mesurer sans agents ni publication :

```text
cargo build --workspace --locked -j 1
python -B scripts/v2_baseline.py benchmark --binary target/debug/factory.exe --build-profile debug --samples 9 --history 1000 --output .factory/v2/p00/backend-benchmark.json
```

Sur Unix, remplacer `target/debug/factory.exe` par `target/debug/factory`. `--build-profile` étiquette la compilation fournie ; il ne transforme pas un build debug en build release. Pour mesurer une release, la construire explicitement puis fournir son chemin et `--build-profile release`. Aucune mesure debug n'est présentée comme une mesure du binaire distribué.

Le script crée son propre dépôt Git temporaire sous le répertoire de sortie, initialise ASF, restaure uniquement sa fixture et porte son historique synthétique à 1 000 workflows. Les deux workflows riches contiennent quatre tâches, des tentatives/sessions et les autres historiques ; les 998 workflows ajoutés n'ont pas de tâches. Cela mesure la croissance d'un historique, pas la navigation dans un grand dépôt de sources ni l'exécution de 1 000 missions.

Pour chaque échantillon, le script mesure une initialisation neuve, lance un nouveau backend sur loopback/port éphémère, attend health, vérifie les refus HTTP, initialise une session puis mesure les lectures. Les lectures ont un échauffement par endpoint. Les requêtes sont séquentielles, avec un nouveau transport HTTP Python par requête ; elles incluent réseau local, sérialisation et lecture complète du corps. Le timeout de sonde de readiness est 50 ms, suivi de 10 ms d'attente en cas d'échec : le démarrage observé est une borne incluant ce polling, pas une mesure interne précise du runtime.

Les caches OS ne sont pas vidés. Aucune mémoire maximale ni interaction de navigateur n'est mesurée. Les enfants démarrés par le script sont terminés et attendus ; les répertoires temporaires dont il est propriétaire sont nettoyés. Le script ne modifie pas la DB, les agents ou les worktrees du projet utilisateur. Les vérifications 401/403 ne provoquent aucune invocation d'agent.

Le JSON conserve les neuf valeurs brutes, médiane, p95 au rang supérieur `ceil(0,95*n)` (donc maximum pour n=9), min/max, taille des réponses, version/hash du binaire, hash de fixture, plateforme et protocole. Un HTTP inattendu, une DB invalide ou un mauvais projet fait échouer la mesure ; aucune valeur de remplacement n'est produite.

## Résultats mesurés

Mesure finale locale du 27 octobre : neuf échantillons, binaire debug reconstruit/vérifié avec Cargo, 1 000 workflows synthétiques. JSON complet (historical record held privately), incluant les neuf valeurs de chaque métrique et les tailles de réponses. Aucun test ASF ni frontend n'était lancé pendant cette mesure finale ; d'autres applications de la machine restent présentes.

| Opération | Médiane (ms) | p95 (ms) |
| --- | ---: | ---: |
| Initialisation ASF neuve | 118,78 | 144,22 |
| Processus → health observé | 244,34 | 263,76 |
| Bootstrap authentifié | 15,51 | 21,09 |
| Liste paginée de 25 runs | 14,35 | 25,81 |
| Liste historique non paginée de 1 000 runs | 30,02 | 39,49 |
| Détail du run riche | 15,33 | 27,11 |
| Graphe des 1 000 runs et tâches de fixture | 261,37 | 320,51 |
| Évaluation des agents de fixture | 14,32 | 26,06 |

Ces résultats décrivent le protocole HTTP local ci-dessus. Les valeurs faibles/fortes par échantillon restent consultables ; neuf observations ne qualifient ni un percentile de production ni une machine sans charge. L'essai initial sous tests concurrents et avec un autre timeout de readiness reste dans `.factory/v2/p00/backend-benchmark.json` ; il n'est pas comparé comme un gain de performance applicative.

Le diagnostic de contexte V1 (historical record held privately), explicitement lancé en plus des gates ordinaires, a mesuré une observation par taille :

| Fixture synthétique | Indexation initiale (ms) | Revérification inchangée (ms) |
| --- | ---: | ---: |
| 100 petits fichiers Rust | 200,42 | 73,06 |
| 1 000 petits fichiers Rust | 1 358,50 | 1 927,60 |

Commande : `cargo test --workspace -j 1 refresh_performance_diagnostic -- --ignored --nocapture`. Le test frontend standard était actif pendant ce diagnostic. Toutes les sources éligibles sont relues et hashées, même inchangées ; il s'agit de mesures ponctuelles, sans médiane/p95 ni seuil de régression. Le benchmark d'évaluation jusqu'à 100 000 tentatives n'a pas été lancé.

L'inventaire de build (historical record held privately) contient les versions verrouillées (React 18.3.1, Vite 6.4.3, TypeScript 5.7.3, React Flow 12.11.3, xterm 6.0.0), les hashes des lockfiles et les tailles/hashes des cinq assets. Les tailles gzip sont dérivées avec `gzip.compress(bytes, mtime=0)` de Python ; elles ne mesurent pas un transfert réseau réel. Le chunk principal de 520,90 kB reste un risque de chargement à mesurer dans le navigateur avant de définir les seuils V2.

## CI et limites de qualification

La CI du commit exact de référence a été relue via le connecteur GitHub le 27 octobre : [run 40383707233](https://github.com/OmegaMc1331/agentic-software-factory/actions/runs/40383707233), neuf jobs terminés avec succès. Le snapshot réduit (historical record held privately) conserve les IDs et conclusions : Rust Windows/Linux/macOS, checksums sur les trois OS, frontend, embedded smoke, installateurs. Il valide la référence, pas les nouveaux commits P00 non publiés.

Les résultats locaux consolidés et le passage de gate P00 sont dans V2-P00.md (historical record held privately). Les tests natifs Linux/macOS de ces ajouts et leur nouvelle CI ne peuvent pas être prétendus exécutés localement sous Windows.

La V1 ne possède ni onglets IDE restaurables, chargement de documents éditables, serveurs LSP/DAP, extensions IDE, scheduler parallèle intra-mission, ni reprise V2. Ces mesures restent indisponibles en P00. Réaction UI/rendu navigateur, consommation mémoire, débit PTY, endurance et agents réels restent non mesurés par cet outil. Ils devront être mesurés avec les prototypes et l'application correspondante avant P14 ; aucun seuil V2 n'est fixé sans ces données.
