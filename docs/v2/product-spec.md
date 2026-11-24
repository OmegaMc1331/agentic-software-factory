# Agentic Software Factory V2.0
## Spécification produit, UX/UI et plan d'implémentation

**Statut :** spécification de référence — conception validée, implémentation non commencée
**Référence technique :** `580eeaa29117edc7c2a6255341483dc798890f11`
**Plateforme :** application locale Rust avec interface web desktop-first
**Périmètre :** toutes les capacités définies dans ce document sont obligatoires avant la release V2.0.

---

# 1. Vision produit

## 1.1 Positionnement

Agentic Software Factory V2 est un environnement de développement dans lequel l'utilisateur délègue des objectifs logiciels à un orchestrateur IA, tout en conservant l'accès direct à son code et le contrôle des opérations importantes.

ASF V2 réunit trois modes d'utilisation dans une même interface :

1. **Assistant de développement :** discuter avec l'orchestrateur, poser des questions, demander des modifications et examiner les résultats.
2. **Environnement IDE :** consulter, modifier, rechercher, exécuter, tester et déboguer du code.
3. **Software factory autonome :** planifier et exécuter des missions multi-agents, suivre leur progression, gérer les reprises et intégrer les modifications.

Ces trois usages ne doivent pas être séparés en trois applications.

## 1.2 Différenciation par rapport à la V1

La V1 est principalement une interface de supervision d'un moteur de workflows.

La V2 adopte une architecture UX centrée sur un **workspace persistant**.

| V1 | V2 |
|---|---|
| Navigation entre des vues de dashboard | Espace de travail unique et modulable |
| Workflow comme objet principal de l'interface | Projet, conversation et mission comme objets de premier niveau |
| Plan DAG exposé directement | Progression concise, détails accessibles à la demande |
| Agent Graph dominant visuellement | Orchestrateur central, agents secondaires |
| Console d'agent | Véritable espace IDE |
| Configuration principalement technique | Configuration progressive avec réglages avancés |
| Plusieurs workflows, exécution interne séquentielle | Orchestration dynamique des tâches parallèles et séquentielles |
| Fermeture du navigateur sans effet sur le runtime | Suspension contrôlée des missions associées |
| Contexte indexé | Contexte enrichi par une mémoire persistante structurée |

## 1.3 Principes non négociables

- L'interface doit ressembler à un véritable outil de développement, pas à une démonstration d'IA.
- Le chat doit être lisible et ne pas devenir un terminal déguisé.
- La complexité technique est accessible, mais ne domine pas l'écran par défaut.
- L'autonomie de l'orchestrateur ne doit jamais contourner les politiques de sécurité.
- Toute opération susceptible d'avoir un effet externe doit respecter une autorisation explicite ou une politique préalablement autorisée.
- Les données restent locales par défaut.
- Les fonctionnalités de la V1 ne doivent pas disparaître sans remplacement fonctionnel.
- Les changements de V2 sont développés par phases internes, mais la publication est bloquée tant que le périmètre complet n'est pas terminé.
- Aucun composant factice, compteur simulé ou écran purement décoratif ne peut être déclaré fonctionnel.

---

# 2. Utilisateurs et parcours essentiels

## 2.1 Utilisateur principal

Développeur individuel utilisant des outils de programmation IA tels que Codex, Claude Code, OpenCode, Gemini CLI ou Qwen Code.

Il peut vouloir écrire lui-même du code, déléguer la totalité d'une tâche ou alterner entre les deux.

L'application doit également convenir à la maintenance de plusieurs projets logiciels en parallèle.

## 2.2 Parcours utilisateur A — Ouvrir et reprendre un projet

1. L'utilisateur démarre ASF.
2. Le dernier workspace ouvert est restauré.
3. Le dernier projet et sa session active sont sélectionnés.
4. Les onglets, panneaux, fichiers et dispositions sont restaurés.
5. Les conversations et missions précédentes sont disponibles.
6. Une mission suspendue peut être reprise depuis son dernier état durable.
7. Si le dernier contexte ne peut pas être restauré, ASF explique précisément pourquoi et propose une récupération.

**Critère :** l'utilisateur retrouve son travail sans devoir reconstruire son interface ni répéter son objectif.

## 2.3 Parcours utilisateur B — Déléguer une mission

1. L'utilisateur écrit un objectif dans la conversation centrale.
2. ASF identifie le projet et le contexte concernés.
3. L'orchestrateur analyse la demande et établit un plan interne.
4. Il choisit les agents autorisés et les dépendances.
5. Il démarre automatiquement si la politique le permet.
6. Le panneau Progression indique l'étape, l'état global et l'objectif attendu.
7. Les agents travaillent et produisent des preuves vérifiables.
8. L'orchestrateur ajuste le plan si nécessaire.
9. ASF présente une synthèse, les tests et les modifications.
10. L'intégration ou la publication suit la politique du projet.

**Critère :** aucune approbation systématique du plan n'est imposée pour une mission ordinaire.

## 2.4 Parcours utilisateur C — Intervenir pendant une mission

1. Une mission est en cours.
2. L'utilisateur ouvre le panneau Agents.
3. Il sélectionne une tâche et choisit un autre agent autorisé.
4. ASF attend ou établit un checkpoint sécurisé.
5. La tâche est reprise par le nouvel agent avec un contexte fidèle.
6. Les tentatives précédentes restent consultables.

**Critère :** aucun travail validé n'est perdu et deux agents ne modifient pas simultanément le même worktree sans coordination.

## 2.5 Parcours utilisateur D — Développer manuellement

1. L'utilisateur ouvre l'explorateur de fichiers.
2. Il ouvre plusieurs fichiers dans des onglets.
3. Il modifie son code dans l'éditeur.
4. Les diagnostics LSP et la navigation sont disponibles.
5. Il lance une commande dans un terminal autorisé.
6. Il peut démarrer une session de débogage.
7. Le panneau Source Control affiche les modifications.
8. Il peut demander à l'orchestrateur de poursuivre une modification à partir du code actuel.

**Critère :** le développement manuel reste utilisable même lorsqu'aucune mission IA n'est active.

## 2.6 Parcours utilisateur E — Travailler sur plusieurs projets

1. L'utilisateur ouvre deux dépôts distincts.
2. Chaque projet possède ses fichiers, conversations, missions, politiques et mémoire.
3. Les onglets des projets restent accessibles dans le workspace.
4. Le passage d'un onglet à l'autre ne suspend aucune mission.
5. Fermer une session suspend les missions qui lui appartiennent.
6. Fermer le dernier client navigateur entraîne la suspension contrôlée des missions concernées.
7. Aucun état sensible d'un projet n'est injecté dans un autre sans action explicite.

---

# 3. Architecture de l'expérience utilisateur

## 3.1 Structure principale

ASF V2 utilise une interface composée de six zones.

**A. Barre supérieure**

Contient le nom du projet, les onglets, la commande de changement de projet, les actions globales et les indicateurs d'état essentiels.

**B. Rail d'activité**

Accès compact aux grandes fonctions : Explorateur, Recherche, Source Control, Agents, Extensions, Mémoire et Paramètres.

**C. Panneau contextuel gauche**

Affiche les éléments de la fonction sélectionnée : fichiers, changements Git, agents, résultats de recherche, etc.

**D. Zone de travail centrale**

Présente l'orchestrateur par défaut. Elle peut aussi accueillir un éditeur de code, une revue, un visualiseur, un débogueur ou des vues fractionnées.

**E. Panneau contextuel droit**

Accueille principalement Progression, Contexte et Agents. Il est facultatif et entièrement rétractable.

**F. Panneau inférieur**

Regroupe Terminal, Logs, Diagnostics, Tests et Sorties des outils. Il peut être redimensionné, masqué ou détaché dans un groupe de vues compatible.

## 3.2 Disposition par défaut

```text
┌─────────────────────────────────────────────────────────────────┐
│ ASF  │ Projet A ▾ │ Session 1 │ Projet B │ + │ Recherche │ ⋯  │
├──────┬───────────────┬───────────────────────────────┬──────────┤
│      │               │                               │          │
│  E   │ Explorateur   │        Orchestrateur          │ Mission  │
│  X   │               │                               │          │
│  P   │ src/          │ Conversation                   │ En cours │
│  L   │ tests/        │                               │ Étape 3/6│
│  O   │ config/       │ Messages et résultats         │          │
│  R   │               │                               │ Détails  │
│  E   │               │                               │          │
│  R   │               │ Composer une demande          │          │
│      │               │                               │          │
├──────┴───────────────┴───────────────────────────────┴──────────┤
│ Terminal │ Diagnostics │ Tests │ Logs                           │
└─────────────────────────────────────────────────────────────────┘
```

Ce schéma indique uniquement les relations entre les zones. Il ne constitue pas une maquette graphique définitive.

## 3.3 Système de panneaux

Fonctions obligatoires :

- Redimensionnement par glisser-déposer.
- Déplacement d'onglets entre groupes.
- Division horizontale et verticale.
- Déplacement d'une vue compatible vers un autre panneau.
- Fermeture et réouverture d'un panneau.
- Maximisation temporaire.
- Sauvegarde automatique de la disposition.
- Restauration après redémarrage.
- Retour à la disposition par défaut.
- Gestion du focus clavier.
- Préservation des états des éditeurs ouverts.

La disposition doit être représentée par un arbre de groupes et de divisions versionné, et non par des positions CSS arbitraires.

## 3.4 Navigation

La navigation principale suit trois niveaux :

**Projet → Session → Vues de travail**

Une session appartient à un projet et contient une conversation, ses missions associées et son état de travail.

Les fichiers, éditeurs et outils sont des vues de travail pouvant être ouvertes dans plusieurs groupes.

Les projets ne doivent pas devenir de simples routes vers des dashboards indépendants.

## 3.5 Onglets

Chaque onglet possède une identité stable et un type :

- Projet/session.
- Conversation.
- Fichier.
- Diff.
- Terminal.
- Débogage.
- Revue.
- Mission.
- Outil ou extension compatible.

Le comportement doit être clair lorsqu'un onglet est déplacé, dupliqué, fermé ou restauré.

Fermer un onglet de fichier ne ferme pas la session. Fermer explicitement une session peut suspendre ses missions.

---

# 4. Inventaire des écrans et vues

## 4.1 Vues obligatoires

| ID | Vue | Fonction principale |
|---|---|---|
| UI-01 | Workspace | Conteneur principal de l'expérience |
| UI-02 | Sélecteur de projets | Ouvrir, créer, importer ou reprendre un projet |
| UI-03 | Orchestrateur | Conversation centrale et création de demandes |
| UI-04 | Nouvelle mission | Chat, formulaire ou plan visuel |
| UI-05 | Progression | État synthétique et accès au détail des missions |
| UI-06 | Agents | État, configuration et remplacement d'agents |
| UI-07 | Explorateur | Navigation et opérations sur les fichiers |
| UI-08 | Éditeur | Modification de code, navigation et diagnostics |
| UI-09 | Recherche globale | Recherche de fichiers, symboles et contenu |
| UI-10 | Source Control | Git, branches, diffs, conflits et historique |
| UI-11 | Revue | Résultats, preuves, modifications et validation |
| UI-12 | Terminal | Shells autorisés et consoles d'agents |
| UI-13 | Débogage | Breakpoints, variables, pile et exécution DAP |
| UI-14 | Extensions | Installation, activation et configuration |
| UI-15 | Mémoire | Connaissances persistantes du projet |
| UI-16 | Historique | Missions, conversations, événements et décisions |
| UI-17 | Paramètres | Agents, modèles, permissions et préférences |
| UI-18 | Notifications | Événements, erreurs et demandes d'intervention |

Ces vues ne doivent pas toutes être des pages plein écran. La plupart sont des panneaux ou des onglets hébergés par le workspace.

## 4.2 Orchestrateur — UI-03

### Structure

- En-tête avec projet, session et statut.
- Historique des messages.
- Messages de l'utilisateur.
- Réponses synthétiques de l'orchestrateur.
- Résultats terminés.
- Références vers les fichiers modifiés.
- Liens internes vers missions et revues.
- Champ de saisie principal.
- Pièces jointes ou références de fichiers sélectionnées.
- Actions contextuelles.

### Comportement

Le chat ne doit pas afficher systématiquement :

- La sortie brute de chaque outil.
- Tous les messages internes des agents.
- Chaque commande shell.
- Toutes les transitions du DAG.

Ces informations appartiennent aux vues de détail.

Le chat peut présenter des messages comme :

« La modification est terminée. Les tests passent. Trois fichiers ont été modifiés. »

Chaque déclaration importante doit être liée aux preuves réelles du runtime.

### Interactions

- Envoyer une demande.
- Ajouter un fichier au contexte.
- Faire référence à un symbole ou une sélection de code.
- Demander une modification.
- Demander des explications.
- Consulter le résultat d'une mission.
- Ouvrir la revue.
- Interrompre ou suspendre une mission.
- Reprendre un travail suspendu.

Les réponses ne doivent pas inventer de progrès en l'absence de données d'exécution.

## 4.3 Nouvelle mission — UI-04

Trois méthodes obligatoires :

**Mode Chat**

Une demande en langage naturel déclenche la planification puis l'exécution.

**Mode Formulaire**

L'utilisateur précise :

- Objectif.
- Projet et périmètre.
- Contraintes.
- Critères d'acceptation.
- Agents préférés, si besoin.
- Profil de permissions.
- Limites d'exécution.
- Politique de livraison.

**Mode Plan visuel**

L'utilisateur peut créer ou modifier un graphe de tâches et leurs dépendances.

Le graphe doit utiliser le modèle de planification réel, pas une représentation décorative indépendante.

Les trois méthodes produisent un contrat de mission commun.

## 4.4 Progression — UI-05

Par défaut, le panneau affiche seulement :

- Intitulé de la mission.
- Statut global.
- Étape actuelle.
- Avancement basé sur les tâches réelles.
- Résultat attendu.
- Intervention nécessaire, le cas échéant.

Une action « Détails » ouvre :

- DAG.
- Tâches et dépendances.
- Agents responsables.
- Tentatives.
- Checkpoints.
- Événements.
- Tests.
- Artefacts.
- Décisions de replanification.

Ne pas inventer un pourcentage d'avancement à partir d'une estimation arbitraire. Si la mesure est impossible, afficher les étapes achevées et restantes.

## 4.5 Agents — UI-06

Le panneau Agents est facultatif et indépendant du chat.

Il montre :

- Agents configurés.
- Agent sélectionné pour une tâche.
- Rôle et modèle.
- État disponible, occupé, indisponible ou en erreur.
- Capacité et concurrence autorisées.
- Tâches assignées.
- Historique et statistiques.
- Permissions effectives.
- Actions de remplacement, diagnostic et configuration.

L'utilisateur peut créer un profil d'agent avec instructions, rôle, outils, modèle et permissions.

Les profils sont validés avant leur utilisation.

## 4.6 Explorateur et éditeur — UI-07 / UI-08

### Explorateur

Fonctions minimales obligatoires :

- Arborescence de fichiers.
- Création, renommage, déplacement et suppression.
- Recherche rapide.
- Affichage des fichiers modifiés.
- Distinction des dossiers et worktrees.
- Menus contextuels.
- Gestion des fichiers binaires.
- Prévention des accès hors périmètre.

### Éditeur

Fonctions attendues :

- Coloration syntaxique.
- Sélection multiple.
- Recherche/remplacement.
- Undo/redo.
- Indentation et formatage.
- Diff côte à côte.
- Navigation entre fichiers.
- Onglets multiples.
- Diagnostics.
- Autocomplétion.
- Hover.
- Définition et références.
- Renommage de symbole lorsque le serveur LSP le permet.
- Organisation des imports lorsque disponible.
- Sauvegarde fiable.
- Détection des modifications externes.
- Résolution des conflits entre buffers et fichiers.

Les langages sont pris en charge au moyen d'adaptateurs et de serveurs de langage disponibles, sans architecture privilégiant artificiellement Rust.

Pour la qualification V2, il faut au minimum démontrer des parcours fonctionnels avec Rust, TypeScript et Python, tout en maintenant une architecture extensible à d'autres langages.

## 4.7 Source Control — UI-10

Inspiré des interactions usuelles d'un IDE.

Fonctions :

- Fichiers modifiés.
- Diffs.
- Stage/unstage.
- Commits.
- Branches.
- Historique.
- Worktrees.
- Conflits de fusion.
- Statut GitHub.
- Pull requests.
- Résultats des revues.

Distinguer visuellement :

1. Changements effectués manuellement.
2. Changements effectués par les agents.
3. Changements déjà intégrés.
4. Changements encore en attente de revue.

Aucune opération distante ne doit être implicitement assimilée à une opération locale.

## 4.8 Revue — UI-11

La revue doit être facultative dans le parcours ordinaire, mais pleinement accessible.

La vue complète présente :

- Objectif initial.
- Résumé du travail.
- Tâches terminées.
- Agents impliqués.
- Changements de fichiers.
- Diffs.
- Tests réellement exécutés.
- Résultats de revue.
- Erreurs ou limitations.
- Branche d'intégration.
- État de publication éventuel.

Le parcours simplifié conserve une action claire pour appliquer, accepter ou publier selon la politique.

Une revue automatique favorable ne prouve pas qu'un travail est correct : les preuves et les résultats restent consultables.

## 4.9 Terminal, débogage et extensions — UI-12 à UI-14

### Terminal

Le terminal de développement est distinct de la console interactive d'un agent.

Le backend doit gérer explicitement les shells autorisés, leur répertoire de travail, leurs sessions et leurs permissions.

Un simple composant xterm.js ne constitue pas à lui seul un terminal système.

### Débogage

Capacités obligatoires :

- Configuration des profils de débogage.
- Lancement et attachement, si l'adaptateur le permet.
- Breakpoints.
- Continue/pause/step.
- Pile d'appels.
- Variables.
- Console de débogage.
- Diagnostics d'échec.

Les fonctionnalités dépendant d'un adaptateur absent doivent être indiquées comme indisponibles, jamais simulées.

### Extensions

ASF doit prendre en charge un mécanisme d'extensions réel.

Il doit permettre au minimum :

- Découverte ou importation depuis une source compatible autorisée.
- Installation.
- Activation/désactivation.
- Désinstallation.
- Affichage de version et permissions.
- Gestion des incompatibilités.
- Gestion des échecs de chargement.

La compatibilité avec les extensions VS Code n'est pas présumée. Elle dépendra de la fondation IDE choisie et de l'évaluation technique.

## 4.10 Mémoire — UI-15

La mémoire de projet est structurée en catégories :

- Architecture.
- Conventions.
- Décisions.
- Contraintes.
- Préférences du projet.
- Contexte métier.
- Informations de maintenance.

Chaque entrée contient au minimum :

- Identifiant.
- Projet.
- Catégorie.
- Contenu.
- Provenance.
- Date de création.
- Date de modification.
- Version.
- Statut de validation.

Fonctions :

- Consultation.
- Recherche.
- Ajout.
- Modification.
- Suppression.
- Historique des changements.
- Restauration d'une version si elle existe.
- Exclusion du contexte d'une mission.
- Contrôle des informations utilisées par les agents.

Les propositions produites par une IA doivent être distinguées des informations vérifiées.

## 4.11 Notifications et paramètres — UI-17 / UI-18

Les notifications sont regroupées dans un centre dédié.

Catégories :

- Mission terminée.
- Intervention demandée.
- Échec.
- Dépassement de limite.
- Conflit Git.
- Agent indisponible.
- Résultat de test.
- Publication terminée ou incertaine.

Les notifications système restent optionnelles et soumises à autorisation du navigateur ou du système.

Les paramètres sont regroupés en :

- Interface.
- Projets.
- Sessions.
- Agents.
- Modèles.
- Permissions.
- Orchestration.
- IDE.
- Extensions.
- Git/GitHub.
- Mémoire.
- Notifications.
- Données et diagnostics.

---

# 5. Design system

## 5.1 Direction esthétique

**Minimaliste premium, technique, calme et précis.**

ASF V2 doit posséder le caractère d'un outil professionnel utilisé pendant plusieurs heures par jour.

Éviter :

- Gradients omniprésents.
- Gros blocs décoratifs.
- Icônes colorées sans fonction.
- Cartes imbriquées inutilement.
- Effets de verre.
- Néons.
- Orbes et particules.
- Ombres excessives.
- Titres démesurés.
- Textes promotionnels dans l'interface de travail.
- Animations permanentes des éléments non actifs.

Le style doit émerger de la qualité des proportions, des espacements, de la typographie et des interactions.

## 5.2 Palette proposée

Ces couleurs sont des valeurs de départ, à valider sur des maquettes réelles et par tests de contraste.

| Token | Mode sombre | Mode clair |
|---|---|---|
| `surface.canvas` | `#111318` | `#FAFBFC` |
| `surface.panel` | `#171A20` | `#FFFFFF` |
| `surface.elevated` | `#1D222A` | `#F2F5F8` |
| `text.primary` | `#E9EDF2` | `#1B2430` |
| `text.secondary` | `#A2ABB7` | `#586474` |
| `border.subtle` | `#2B313B` | `#E0E5EB` |
| `accent.blue` | `#7BA4ED` | `#376FCC` |
| `accent.hover` | `#94B6F2` | `#285BAF` |
| `state.success` | `#65B58B` | `#23825B` |
| `state.warning` | `#D7AC68` | `#966212` |
| `state.error` | `#DF8585` | `#BD4444` |

Les codes de couleur ne doivent pas être utilisés directement dans les composants.

Employer des tokens sémantiques tels que :

- `color.background.primary`
- `color.surface.secondary`
- `color.text.muted`
- `color.border.default`
- `color.action.primary`
- `color.state.danger`

## 5.3 Typographie

Direction proposée :

- **Interface :** Inter ou une alternative équivalente, à confirmer lors du prototypage.
- **Code et valeurs techniques :** JetBrains Mono ou une alternative monospace lisible.

Hiérarchie indicative :

| Usage | Taille |
|---|---|
| Texte d'interface | 13–14 px |
| Texte de conversation | 14–15 px |
| Titre de panneau | 13–14 px |
| Titre de section | 16–18 px |
| Code | 12–14 px |
| Métadonnées | 11–12 px |

Éviter d'utiliser systématiquement une graisse 700. Préférer les contrastes de poids, de teinte et d'espacement.

## 5.4 Espacements et formes

Échelle proposée :

`4 / 8 / 12 / 16 / 20 / 24 / 32 px`

Principes :

- Panneaux séparés principalement par des lignes de 1 px.
- Rayons faibles ou moyens selon les composants.
- Grandes surfaces globalement plates.
- Densité intermédiaire.
- Aucun panneau ne doit gaspiller de l'espace vertical par des titres décoratifs.

Les tailles doivent être adaptées aux vrais contenus de développement : noms de fichiers longs, logs, diffs, arbres profonds et messages volumineux.

## 5.5 Iconographie

Utiliser une famille d'icônes cohérente.

Les icônes servent à identifier des actions ou des concepts, pas à décorer l'interface.

Distinguer :

- Action.
- Navigation.
- Type de fichier.
- État.
- Avertissement.
- Statut d'exécution.

Les états critiques ne doivent jamais être indiqués uniquement par une couleur.

## 5.6 Micro-interactions

Transitions recommandées :

- Retour d'action : 100–140 ms.
- Ouverture de panneau : 160–220 ms.
- Modification de disposition : 180–260 ms.
- Apparition de notification : 150–220 ms.

Ces durées sont indicatives, pas des animations obligatoires.

Respecter `prefers-reduced-motion`.

Pendant les longues opérations :

- Ne pas faire clignoter les composants.
- Ne pas afficher de pourcentage fictif.
- Utiliser des statuts textuels précis.
- Garder les interactions disponibles lorsqu'elles sont sûres.

## 5.7 Accessibilité

Objectif : WCAG 2.2 AA pour les interfaces applicables.

Exigences :

- Contrastes vérifiés.
- Focus clavier visible.
- Navigation cohérente.
- Libellés accessibles.
- États non exprimés uniquement par la couleur.
- Raccourcis non conflictuels.
- Animations réduites.
- Redimensionnement et zoom utilisables.
- Lecteurs d'écran pris en compte pour les actions principales.
- Alternatives textuelles aux graphes de tâches.

## 5.8 Dispositions prédéfinies

Fournir au minimum quatre presets :

**Focus** — orchestrateur central, peu de panneaux.

**Code** — explorateur et éditeur dominants, orchestrateur accessible à côté.

**Agents** — conversation et supervision multi-agent.

**Review** — diffs, changements Git, tests et résultat final.

Chaque preset reste personnalisable. Le dernier état est restauré automatiquement.

---

# 6. Architecture technique cible

## 6.1 Fondations conservées

Conserver et faire évoluer les systèmes existants :

- Rust workspace.
- Factory Core.
- Factory Runtime.
- SQLite.
- Factory Policy.
- Factory Context.
- Git worktrees.
- Factory GitHub.
- Factory MCP.
- Factory Evaluation.
- API locale.
- Frontend React/TypeScript/Vite, sauf justification technique documentée.

La migration vers V2 ne doit pas supprimer les garanties des campagnes de stabilisation précédentes.

## 6.2 Changement architectural principal : multi-projet

La V1 est largement organisée autour d'une racine de dépôt et de son dossier `.factory/`.

La V2 doit permettre plusieurs projets ouverts dans une même instance de l'application.

Architecture proposée :

**Service local ASF**

Gère :

- Registre des projets.
- Sessions de l'interface.
- Préférences globales.
- Notifications.
- Connexions des clients navigateur.
- Cycle de vie des runtimes.

**Contexte de projet isolé**

Gère :

- Racine du dépôt.
- Base SQLite existante.
- Configuration.
- Workflows.
- Worktrees.
- Agents et politiques effectives.
- Mémoire du projet.
- Sessions de travail.

Chaque requête doit être associée à un projet identifié et autorisé.

Ne pas mélanger les configurations de deux projets par simple mutation d'une variable globale de racine.

## 6.3 Frontend

Architecture de composants proposée :

- `AppShell`
- `ProjectSwitcher`
- `TabManager`
- `DockManager`
- `ConversationView`
- `MissionComposer`
- `MissionProgress`
- `AgentPanel`
- `FileExplorer`
- `CodeEditor`
- `SourceControl`
- `DiffViewer`
- `TerminalPanel`
- `DebugPanel`
- `ExtensionsPanel`
- `MemoryPanel`
- `NotificationCenter`
- `SettingsWorkspace`

La structure exacte des packages reste à déterminer après l'audit de dépendances.

Favoriser des composants accessibles, testables et relativement indépendants de l'état du moteur d'exécution.

## 6.4 Couche applicative

La couche frontend ne doit jamais devenir propriétaire de l'exécution réelle.

Le backend Rust reste responsable :

- Des processus.
- Des opérations Git.
- Des permissions.
- Des checkpoints.
- Du stockage durable.
- De l'ordonnancement.
- Des transitions de mission.
- Des fichiers.
- Du terminal.
- Des serveurs LSP/DAP.
- Des extensions côté serveur lorsque nécessaires.

Le frontend représente les états et transmet des intentions explicites.

## 6.5 Transport et événements

Utiliser les transports appropriés selon leur fonction :

- HTTP pour les commandes et lectures ordinaires.
- SSE pour les événements serveur vers client.
- WebSocket pour les interactions bidirectionnelles comme le terminal et les protocoles IDE lorsque nécessaire.

Les événements d'exécution doivent avoir :

- Identifiant durable.
- Numéro de séquence ou curseur.
- Projet.
- Session.
- Mission.
- Type.
- Horodatage.
- Données typées.
- Version de schéma.

À la reconnexion, le client récupère les événements manquants plutôt que de supposer qu'aucun événement ne s'est produit.

Les commandes modifiant l'état doivent être idempotentes ou protégées contre les doublons.

## 6.6 Modèle de données V2

Entités nouvelles ou enrichies proposées :

- `ProjectRegistryEntry`
- `Workspace`
- `WorkspaceLayout`
- `WorkspaceTab`
- `Conversation`
- `ConversationMessage`
- `Mission`
- `MissionEvent`
- `MissionCheckpoint`
- `ExecutionLease`
- `AgentProfile`
- `AgentModelConfiguration`
- `ProjectPermissionPolicy`
- `ProjectMemoryEntry`
- `ProjectMemoryRevision`
- `Notification`
- `EditorDocumentState`
- `DebugConfiguration`
- `ExtensionInstallation`

Ce sont des contrats de conception proposés, pas des tables prétendument déjà présentes.

Réutiliser les entités V1 existantes dès qu'elles ont une sémantique équivalente.

Éviter de dupliquer les workflows, tâches, tentatives et sessions existants sous de nouveaux noms uniquement pour la V2.

---

# 7. Orchestration agentique V2

## 7.1 Orchestrateur central

Il doit :

1. Analyser la demande.
2. Construire un plan.
3. Identifier les dépendances.
4. Choisir les agents autorisés.
5. Évaluer la concurrence possible.
6. Démarrer le travail.
7. Suivre les résultats.
8. Diagnostiquer les échecs.
9. Modifier le plan si nécessaire.
10. Organiser les revues et l'intégration.
11. Présenter un résultat vérifiable.

Le choix des agents, modèles et outils reste contraint par les capacités réellement disponibles.

## 7.2 Ordonnancement dynamique

Le moteur doit distinguer :

- Tâches indépendantes, parallélisables.
- Tâches dépendantes, séquentielles.
- Tâches conflictuelles, nécessitant une coordination.
- Tâches bloquées par une permission.
- Tâches exigeant une intervention humaine.

L'orchestrateur peut proposer le parallélisme, mais **le scheduler Rust décide si l'exécution est sûre et autorisée**.

## 7.3 Isolation des modifications

Toute tâche concurrente susceptible d'écrire dans le dépôt utilise un espace isolé adapté, principalement un worktree Git.

Les changements doivent passer par un mécanisme d'intégration contrôlé.

Garanties :

- Pas de deux propriétaires concurrents pour un même espace d'écriture non isolé.
- Pas d'écrasement silencieux des modifications.
- Pas de fusion automatique si les invariants de sécurité sont violés.
- Tests et revues associés au bon état Git.
- Gestion des conflits.
- Historique durable des intégrations.

## 7.4 Replanification

ASF peut modifier librement le travail restant, mais ne doit pas réécrire l'historique.

Chaque révision du plan contient :

- Raison.
- Plan précédent.
- Nouveau plan.
- Tâches ajoutées.
- Tâches remplacées.
- Dépendances modifiées.
- Impact sur les tâches en cours.
- Horodatage.

Les tâches terminées restent historiques.

Les tâches exécutées ne doivent pas être modifiées rétroactivement.

## 7.5 Gestion des échecs

La stratégie est :

**Observer → Diagnostiquer → Corriger → Vérifier → Réessayer**

L'orchestrateur doit distinguer :

- Erreur de compilation.
- Échec de test.
- Échec d'agent.
- Échec d'outil.
- Conflit Git.
- Absence de capacité.
- Violation de politique.
- Dépassement de ressource.
- Panne du runtime.

Toutes ces situations ne peuvent pas être traitées comme un simple retry.

Les politiques de sécurité, plafonds de tentatives et limites temporelles restent prioritaires.

## 7.6 Remplacement d'agent

Le remplacement d'un agent actif impose :

1. Demande de transfert.
2. Arrêt de l'attribution de nouveau travail à l'agent.
3. Création ou validation d'un checkpoint.
4. Conservation des artefacts et modifications existantes.
5. Révocation des capacités temporaires devenues inutiles.
6. Vérification des permissions du nouvel agent.
7. Transmission du contexte durable.
8. Reprise contrôlée.

Si un transfert sûr n'est pas possible, ASF doit proposer une reprise depuis un état antérieur, pas prétendre avoir effectué un transfert transparent.

## 7.7 Modèles et fournisseurs

Un modèle ou fournisseur peut être configuré par agent.

ASF reste un orchestrateur de CLI externes par défaut.

La configuration peut contenir :

- Exécutable.
- Fournisseur.
- Profil.
- Identifiant de modèle, si le CLI prend en charge ce réglage.
- Arguments.
- Environnement autorisé.
- Rôle.
- Outils.
- Permissions.
- Limites.

Ne pas présenter une fonctionnalité de sélection de modèle comme disponible si le CLI correspondant ne l'expose pas.

Ne pas collecter ou réinventer la gestion des clés des fournisseurs sans nécessité clairement identifiée.

---

# 8. Sessions, suspension et reprise

## 8.1 Sémantique demandée

Comportements obligatoires :

| Action | Comportement |
|---|---|
| Changer d'onglet dans ASF | La mission continue |
| Ouvrir un autre projet | La mission continue |
| Masquer temporairement le navigateur | La mission continue |
| Fermer un onglet de fichier | La mission continue |
| Fermer explicitement une session | Ses missions sont suspendues |
| Fermer le dernier client navigateur actif | Suspension contrôlée des missions liées |
| Réouvrir ASF | État restauré, reprise possible |
| Redémarrer le serveur Rust après incident | Réconciliation et proposition de reprise |

## 8.2 Détection de fermeture

La fermeture du navigateur ne peut pas être détectée de manière parfaitement fiable par un simple événement JavaScript.

La conception doit donc utiliser :

- Identifiant de client.
- Identifiant de session.
- Heartbeat.
- Lease renouvelable.
- Détection de disparition du dernier client propriétaire.
- Délai de grâce configurable et borné.
- Événement explicite de fermeture lorsque disponible.

La visibilité du document ne doit pas être assimilée à la fermeture du navigateur.

Plusieurs onglets ou fenêtres rattachés à une même session doivent être pris en compte.

## 8.3 Suspension

La suspension doit être une transition durable.

États proposés :

- `active`
- `suspending`
- `suspended`
- `resuming`
- `completed`
- `failed`
- `cancelled`

Les nouveaux états doivent être intégrés au modèle de workflow existant avec une stratégie de migration explicite.

À la suspension :

1. Bloquer l'ordonnancement de nouvelles tâches.
2. Demander l'arrêt coopératif du travail actif.
3. Capturer les informations récupérables.
4. Persister les checkpoints.
5. Terminer les processus qui ne peuvent pas être conservés en sécurité.
6. Préserver worktrees et preuves.
7. Marquer les tâches en cours comme suspendues ou nécessitant une reprise.

Une invocation CLI qui ne prend pas en charge la reprise interne doit être redémarrée à partir d'un checkpoint logique. Ne pas prétendre restaurer sa mémoire volatile.

## 8.4 Reprise

À la reprise :

- Recharger l'état persistant.
- Vérifier le projet et le dépôt.
- Vérifier les fichiers et commits.
- Vérifier les permissions.
- Vérifier les agents disponibles.
- Reconstituer les tâches prêtes.
- Réconcilier les opérations externes incertaines.
- Reprendre uniquement les opérations sûres.

Ne jamais rejouer aveuglément une opération de publication GitHub dont le résultat est incertain.

---

# 9. Mémoire persistante des projets

## 9.1 Objectif

La mémoire permet à l'orchestrateur de réutiliser des décisions et conventions d'une session à l'autre sans demander systématiquement à l'utilisateur de les répéter.

## 9.2 Structure

Chaque information possède une provenance.

Sources possibles :

- Déclaration utilisateur.
- Documentation du dépôt.
- Observation de code.
- Test.
- Décision d'architecture.
- Résultat d'une mission.
- Proposition d'agent.

Une hypothèse d'agent n'est pas automatiquement transformée en vérité durable.

## 9.3 Contrôle utilisateur

L'utilisateur peut :

- Voir toutes les entrées.
- Modifier une entrée.
- Supprimer une entrée.
- Consulter son historique.
- Révoquer une information.
- Exclure une catégorie du contexte.
- Désactiver la mémoire pour un projet.
- Exporter les données du projet.

La suppression doit avoir une sémantique explicite : suppression active, conservation éventuelle d'un historique d'audit et politique de rétention.

## 9.4 Injection contextuelle

La mémoire ne doit pas être injectée intégralement à chaque mission.

Sélectionner uniquement les entrées pertinentes selon :

- Projet.
- Type de tâche.
- Rôle.
- Fiabilité.
- Fraîcheur.
- Budget de contexte.

Les règles de permissions continuent de s'appliquer.

---

# 10. Autorisations et sécurité

## 10.1 Politique par projet

Un projet peut définir :

- Agents autorisés.
- Outils MCP autorisés.
- Périmètres de fichiers.
- Commandes.
- Variables d'environnement.
- Accès réseau déclaré.
- Niveau d'autonomie.
- Limites de concurrence.
- Limites de durée.
- Limites de tentatives.
- Politique Git.
- Politique de publication.

Les politiques doivent être versionnées et auditables.

## 10.2 Opérations sensibles

Distinguer :

**Opérations locales réversibles** — consultation, édition contrôlée, tests, commits locaux.

**Opérations potentiellement destructives** — suppression, nettoyage, reset, écrasement.

**Opérations externes** — push, création de PR, publication, déploiement, action distante.

Les opérations externes sont refusées par défaut tant qu'une politique ou une confirmation explicite ne les autorise pas.

Même avec une autorisation automatique, une action incertaine doit faire l'objet d'une réconciliation avant une éventuelle répétition.

## 10.3 Environnement de confiance

Les worktrees offrent une isolation Git, pas une isolation du système d'exploitation.

Les CLI d'agents et les extensions peuvent disposer des permissions de l'utilisateur qui exécute ASF.

La V2 doit conserver cette distinction dans :

- L'interface.
- La documentation.
- Les configurations de permissions.
- Les messages d'erreur.
- Les audits de sécurité.

Un véritable sandbox OS constituerait une capacité supplémentaire qui exige une validation propre. Il ne faut pas présenter les politiques logiques comme un sandbox.

## 10.4 Sécurité de l'IDE

L'éditeur, les terminaux, serveurs LSP, adaptateurs DAP et extensions introduisent de nouvelles surfaces de risque.

Exigences :

- Accès aux fichiers limité au projet ou au worktree autorisé.
- Vérification des chemins et liens symboliques.
- Permissions spécifiques pour les processus.
- Contrôles avant exécution.
- Limites de durée et de ressources.
- Journalisation des actions sensibles.
- Protection des secrets.
- Authentification et CSRF du serveur local.
- Contrôle des connexions WebSocket.
- Validation de l'origine.
- Aucune exposition automatique du serveur local au réseau.

---

# 11. Choix du socle IDE

## 11.1 Principe

Le choix n'est pas préétabli.

Codex doit mener une évaluation technique reproductible avant de sélectionner la solution.

Candidats à examiner :

- Monaco Editor.
- CodeMirror 6.
- Eclipse Theia.
- Une base VS Code web open source appropriée, notamment les approches de type OpenVSCode Server ou code-server.

Des alternatives peuvent être ajoutées avec justification.

## 11.2 Critères de décision

| Critère | Poids |
|---|---:|
| Qualité de l'édition et ergonomie | 15 % |
| Intégration LSP | 15 % |
| Intégration DAP | 15 % |
| Extensions utilisables et modèle de compatibilité | 15 % |
| Intégration avec le backend Rust | 15 % |
| Performances et mémoire | 10 % |
| Sécurité et maintenance | 10 % |
| Licence et distribution | 5 % |

Les scores doivent s'appuyer sur des prototypes, des mesures et des documentations.

## 11.3 Preuves minimales du prototype

Chaque solution sérieusement retenue doit démontrer :

- Ouverture d'un projet réel.
- Édition et sauvegarde.
- Plusieurs fichiers et vues fractionnées.
- Diagnostic et navigation LSP.
- Débogage DAP sur un projet représentatif.
- Terminal système autorisé.
- Chargement d'une extension compatible ou démonstration de l'API d'extension.
- Fonctionnement avec le backend local Rust.
- Construction et packaging reproductibles.
- Compatibilité avec la navigation ASF prévue.

Documenter explicitement les capacités indisponibles.

## 11.4 Résultat attendu

Livrable :

`docs/v2/ide-evaluation.md`

Il contient :

- Candidats évalués.
- Versions.
- Licences.
- Prototypes.
- Mesures.
- Captures.
- Limitations.
- Risques.
- Tableau de scores.
- Justification de la solution retenue.

La solution sélectionnée doit permettre d'atteindre le périmètre IDE complet avant la publication V2.0.

Il ne faut pas choisir Monaco uniquement parce qu'il ressemble visuellement à VS Code : Monaco seul ne garantit pas la compatibilité avec les extensions VS Code.

---

# 12. Migration depuis ASF V1

## 12.1 Données à préserver

- Base SQLite existante.
- Workflows.
- Tâches.
- Tentatives.
- Sessions d'agents.
- Artefacts.
- Historique des revues.
- Évaluations.
- Configurations d'agents.
- Rôles et assignments.
- Recettes.
- Politiques.
- Intégrations GitHub.
- Données de contexte.
- Worktrees existants.

## 12.2 Stratégie

1. Définir les migrations de schéma versionnées.
2. Prendre une sauvegarde vérifiable avant migration.
3. Conserver les fichiers TOML existants ou fournir une migration explicite.
4. Ajouter les nouvelles données sans supprimer les historiques V1.
5. Tester l'ouverture d'un vrai projet au format V1.
6. Vérifier la reprise et la consultation des anciennes missions.
7. Vérifier les retours arrière sur sauvegarde lorsque possible.

## 12.3 Compatibilité

Le CLI existant doit rester opérationnel ou disposer d'une migration documentée.

Les anciens endpoints HTTP peuvent être maintenus par une couche de compatibilité durant la transition.

Ne pas casser silencieusement les scripts et les usages existants.

La V2 doit pouvoir ouvrir les projets V1 sans demander de recréer les agents, les rôles ou les workflows.

---

# 13. Plan d'implémentation pour Codex

**Principe :** une phase = un objectif précis, des modifications vérifiables, un rapport et des tests de non-régression.

Chaque phase produit une base utilisable pour la suivante.

Aucune phase ne doit être déclarée terminée sur la seule base d'un rendu visuel ou d'une compilation.

## Phase V2-P00 — Audit et baseline

**Objectif :** figer l'état de référence.

Travail :

- Cartographier les composants existants.
- Identifier les capacités réutilisables.
- Inventorier tous les endpoints.
- Inventorier les modèles SQLite.
- Vérifier les garanties de sécurité.
- Mesurer les performances de la V1.
- Créer les tests de référence et fixtures V1.
- Identifier les risques de migration.

Livrables :

- `docs/v2/baseline.md`
- `docs/v2/current-architecture.md`
- `docs/v2/compatibility-matrix.md`

**Gate :** baseline reproductible et CI verte.

## Phase V2-P01 — Évaluation des fondations IDE

**Objectif :** choisir un socle IDE adapté sur preuves.

Travail :

- Comparer les candidats.
- Réaliser des prototypes courts.
- Tester l'éditeur, LSP, DAP, extensions et terminal.
- Vérifier la licence, le packaging et l'architecture Rust.
- Mesurer les coûts de maintenance et de performance.

Livrables :

- `docs/v2/ide-evaluation.md`
- Prototypes isolés.
- Architecture Decision Record.

**Gate :** choix technique démontré et compatible avec les exigences V2.0.

## Phase V2-P02 — Design system

**Objectif :** établir le langage visuel définitif.

Travail :

- Thèmes clair/sombre.
- Tokens.
- Typographie.
- Composants.
- États.
- Focus clavier.
- Animations.
- Gestion des erreurs.
- Système d'icônes.
- Prototypes des principales vues.

Livrables :

- `docs/v2/design-system.md`
- Bibliothèque de composants.
- Maquettes et captures des états.
- Tests de contraste et accessibilité.

**Gate :** cohérence visuelle vérifiée dans les deux thèmes.

## Phase V2-P03 — Runtime multi-projet

**Objectif :** permettre plusieurs projets dans une instance ASF.

Travail :

- Registre global des projets.
- Contextes par projet.
- Ouverture/importation/création.
- Routage des requêtes.
- Isolation des bases.
- Gestion des chemins.
- Verrous et prévention des collisions.
- Migrations et compatibilité V1.

**Gate :** deux projets actifs simultanément sans mélange de données ou d'opérations.

## Phase V2-P04 — Workspace, onglets et panneaux

**Objectif :** remplacer l'ancienne navigation par le workspace.

Travail :

- Shell.
- Onglets déplaçables.
- Panneaux redimensionnables.
- Split views.
- Presets.
- Mémorisation.
- Restauration.
- Gestion du focus et des raccourcis.
- Historique de navigation.

**Gate :** restauration exacte d'un workspace complexe après redémarrage.

## Phase V2-P05 — Système de fichiers et éditeur

**Objectif :** rendre le code éditable dans ASF.

Travail :

- API d'accès contrôlé aux fichiers.
- Explorateur.
- Éditeur choisi.
- Sauvegarde.
- Détection des modifications concurrentes.
- Recherche et remplacement.
- Diffs.
- États des documents.
- Intégration avec les worktrees.

**Gate :** édition fiable de projets réels, sans perte de modifications.

## Phase V2-P06 — Infrastructure IDE avancée

**Objectif :** livrer les capacités IDE complètes.

Travail :

- LSP côté serveur.
- Diagnostics.
- Autocomplétion et navigation.
- DAP.
- Breakpoints et outils de débogage.
- Terminal de développement sécurisé.
- Système d'extensions.
- Gestion des capacités et erreurs des outils externes.
- Tests multiplateformes.

**Gate :** projets réels Rust, TypeScript et Python utilisables avec les capacités disponibles ; LSP, DAP et extensions effectivement opérationnels.

## Phase V2-P07 — Conversation et création de missions

**Objectif :** installer l'orchestrateur au centre du produit.

Travail :

- Conversations persistantes.
- Streaming.
- Historique.
- Pièces jointes et références de code.
- Chat, formulaire et plan visuel.
- Contrat commun de mission.
- Événements liés aux exécutions.
- Panneau Progression.

**Gate :** création et suivi d'une mission réelle depuis les trois modes.

## Phase V2-P08 — Ordonnanceur multi-agent dynamique

**Objectif :** permettre l'exécution parallèle sûre dans une mission.

Travail :

- Scheduler par dépendances.
- Concurrence contrôlée.
- Isolation des tâches.
- Intégration des branches.
- Gestion des conflits.
- Répartition des agents.
- Replanification.
- Stratégies de réparation.
- Historique des décisions.

**Gate :** une mission à plusieurs agents combine tâches parallèles et séquentielles sans corruption ni intégration non validée.

## Phase V2-P09 — Profils d'agents et politiques de projet

**Objectif :** rendre les agents personnalisables et remplaçables.

Travail :

- Profils.
- Instructions.
- Modèles et fournisseurs.
- Outils.
- Permissions.
- Règles d'autonomie.
- Plafonds de ressources.
- Remplacement après checkpoint.
- Historique des transferts.

**Gate :** remplacement d'un agent actif sans perte de l'état validé.

## Phase V2-P10 — Suspension, reprise et résilience

**Objectif :** rendre le cycle de vie conforme aux choix utilisateur.

Travail :

- Leases de navigateur.
- Détection du dernier client.
- Suspension contrôlée.
- Checkpoints.
- Reprise durable.
- Réconciliation après crash.
- Gestion des processus.
- Reconnexion des flux.
- Tests de fermeture brutale du navigateur.

**Gate :** fermeture du navigateur, redémarrage de l'application et reprise sans duplication de tâches ni effet externe accidentel.

## Phase V2-P11 — Mémoire persistante

**Objectif :** donner aux projets une mémoire structurée et maîtrisable.

Travail :

- Schéma.
- Historique.
- Catégories.
- Provenance.
- Modification et suppression.
- Sélection contextuelle.
- Permissions.
- Interface de consultation.
- Tests d'isolation entre projets.

**Gate :** décisions réutilisables entre sessions, sans injection de données d'un autre projet.

## Phase V2-P12 — Git, revue et livraison

**Objectif :** intégrer la gestion du code et des résultats à l'IDE.

Travail :

- Source Control.
- Diffs.
- Branches.
- Commits.
- Worktrees.
- Conflits.
- Revue complète.
- Validation simplifiée.
- Livraison configurable.
- Statuts GitHub.
- Réconciliation des opérations incertaines.

**Gate :** workflow complet objectif → implémentation → revue → intégration → PR autorisée.

## Phase V2-P13 — Finition UX, paramètres et notifications

**Objectif :** terminer la cohérence globale du produit.

Travail :

- Centre de notifications.
- Notifications système optionnelles.
- Paramètres réorganisés.
- Actions clavier.
- États d'erreur.
- Empty states.
- Animations.
- Accessibilité.
- Dispositions.
- Nettoyage des anciennes vues redondantes.

**Gate :** tous les écrans utilisent le design system et disposent de comportements fonctionnels.

## Phase V2-P14 — Validation complète et release candidate

**Objectif :** qualifier l'ensemble de la V2.

Travail :

- Tests Rust.
- Tests frontend.
- Tests d'intégration.
- Tests d'API.
- Tests de sécurité.
- Tests de migration V1.
- Tests IDE réels.
- Missions multi-agents réelles.
- Tests de panne et reprise.
- Tests de concurrence Git.
- Tests d'endurance.
- Validation Windows/Linux/macOS.
- Tests installateurs.
- Checksums.
- Packaging embarqué.
- Audit des dépendances et licences.
- Vérification des fonctionnalités V1.
- Vérification UX/UI des deux thèmes.

**Gate :** tous les critères de release définis à la section 15 sont satisfaits.

---

# 14. Contrat de travail commun à toutes les phases Codex

Chaque phase doit suivre la même structure de livraison.

## 14.1 Avant implémentation

Codex doit :

1. Lire la documentation actuelle.
2. Vérifier le HEAD et l'état Git.
3. Identifier les composants affectés.
4. Préciser les contrats préservés.
5. Écrire les tests ou fixtures nécessaires.
6. Identifier les risques de migration.

## 14.2 Pendant implémentation

Codex doit :

- Réutiliser les capacités existantes.
- Éviter les duplications de logique.
- Ne pas introduire de composants fictifs.
- Préserver les permissions.
- Ne pas masquer les erreurs.
- Ne pas transformer les sorties d'agents en preuves de réussite sans vérification.
- Préserver les données.
- Éviter les refontes hors périmètre de la phase.

## 14.3 Après implémentation

Chaque phase doit livrer :

- Code.
- Tests.
- Documentation.
- Captures ou preuves d'interface lorsque pertinent.
- Rapport des migrations.
- Rapport des risques.
- Résultats de validation.
- Limitations.
- Commits identifiables.

Nom des rapports :

`docs/v2/deliveries/V2-PXX.md`

## 14.4 Interdictions

- Pas de release automatique.
- Pas de suppression des données V1 pour simplifier une migration.
- Pas de désactivation de tests afin d'obtenir une CI verte.
- Pas de mock présenté comme une capacité réelle.
- Pas de suppression des garde-fous GitHub.
- Pas de changement de sécurité non documenté.
- Pas de dépendance IDE choisie uniquement pour son apparence.

---

# 15. Critères d'acceptation de la release V2.0

## 15.1 Couverture fonctionnelle

Aucune release V2.0 n'est autorisée tant que les fonctions suivantes ne sont pas utilisables :

- Multi-projet.
- Multi-session.
- Onglets déplaçables.
- Vues fractionnées.
- Dispositions restaurées.
- Thèmes clair/sombre.
- Orchestrateur conversationnel.
- Trois méthodes de création de mission.
- Progression compacte.
- Panneau Agents.
- Agents personnalisés.
- Modèle par agent.
- Permissions et plafonds par projet.
- Ordonnancement dynamique.
- Tâches parallèles isolées.
- Replanification.
- Réparation et retry.
- Remplacement d'agent par checkpoint.
- Suspension lors de la fermeture.
- Reprise après incident.
- Mémoire persistante structurée.
- Explorateur de fichiers.
- Éditeur de code.
- LSP.
- DAP.
- Terminal.
- Extensions.
- Source Control.
- Revue.
- Livraison configurable.
- Notifications.
- Historique.
- Compatibilité des projets V1.

## 15.2 Tests de missions réelles

Le protocole de qualification doit comprendre au minimum les scénarios suivants :

1. Correction simple dans un dépôt Rust.
2. Développement d'une fonctionnalité TypeScript.
3. Modification d'un projet Python.
4. Mission avec au moins deux tâches indépendantes exécutées en parallèle.
5. Mission combinant tâches parallèles et dépendantes.
6. Échec de compilation, diagnostic et réparation.
7. Échec de test puis replanification.
8. Remplacement d'un agent après checkpoint.
9. Conflit d'intégration Git.
10. Fermeture du navigateur puis reprise.
11. Redémarrage brutal du runtime puis réconciliation.
12. Livraison GitHub autorisée, avec simulation contrôlée d'une issue ou PR incertaine.

Les scénarios doivent utiliser de véritables agents et de vrais dépôts de test.

Les fixtures et mocks restent utilisés pour les tests déterministes, mais ne remplacent pas les validations réelles demandées.

## 15.3 Validation IDE

Démontrer :

- Édition et sauvegarde réelles.
- Recherche globale.
- Diagnostics LSP réels.
- Navigation vers les définitions.
- Débogage DAP réel.
- Terminal réellement connecté.
- Extension réellement chargée.
- Gestion d'un conflit entre édition manuelle et modification d'agent.
- Restauration de fichiers et panneaux.

## 15.4 Validation de la sécurité

- Aucun accès fichier hors périmètre via l'API.
- Aucune opération GitHub non autorisée.
- Aucune fuite de secrets dans les événements.
- Aucune répétition dangereuse d'une publication incertaine.
- Permissions appliquées côté Rust.
- Agents correctement isolés au niveau des worktrees.
- Restrictions du modèle de sécurité documentées.
- Extensions et adaptateurs soumis aux règles de confiance définies.

## 15.5 Performance

Établir des benchmarks reproductibles sur une machine de référence.

Mesurer :

- Démarrage de l'application.
- Ouverture d'un projet.
- Restauration des onglets.
- Chargement d'un fichier.
- Navigation dans un grand dépôt.
- Recherche.
- Temps de réaction de l'interface.
- Mémoire des serveurs LSP.
- Consommation des extensions.
- Flux terminaux.
- Exécution multi-agent.
- Reprise après fermeture.

Définir puis valider des seuils chiffrés à partir de la baseline et des prototypes. Ne pas annoncer des performances non mesurées.

## 15.6 Compatibilité et distribution

La release candidate doit être testée sur :

- Windows x86_64.
- Linux x86_64.
- macOS Apple Silicon.
- macOS Intel.

Valider :

- Installateur.
- Lancement local.
- Authentification du navigateur.
- Chargement des ressources embarquées.
- Fonctionnement du terminal.
- Compatibilité des adaptateurs de développement disponibles.
- Migration depuis V1.
- Intégrité des archives.
- Checksums.
- Réinstallation.
- Mise à jour depuis la dernière release V1.

## 15.7 Décision Go / No-Go

**GO** uniquement si :

- Toutes les fonctionnalités obligatoires sont terminées.
- Aucun défaut P0/P1 ouvert n'est accepté.
- Les défauts P2 sont corrigés ou explicitement évalués comme non bloquants.
- Les régressions V1 sont couvertes.
- Toutes les CI obligatoires réussissent.
- Les missions réelles imposées réussissent.
- Les scénarios de reprise et de conflits réussissent.
- Les fonctionnalités IDE fonctionnent réellement.
- La migration des données est validée.
- Les binaires sont testés sur leurs plateformes.
- Le design final a été vérifié dans les thèmes clair et sombre.

Dans le cas contraire : **NO-GO**, même si l'interface est visuellement terminée.

---

# 16. Livrables documentaires de la conception

La spécification doit devenir le point d'entrée du dossier `docs/v2/`.

Arborescence cible :

```text
docs/v2/
  product-spec.md
  information-architecture.md
  user-flows.md
  design-system.md
  component-inventory.md
  screen-specifications.md
  architecture-target.md
  ide-evaluation.md
  data-model.md
  api-contracts.md
  session-lifecycle.md
  orchestration-design.md
  security-model.md
  migration-v1-v2.md
  implementation-plan.md
  acceptance-matrix.md
  release-gates.md
  deliveries/
    V2-P00.md
    ...
    V2-P14.md
```

Les documents techniques seront affinés après les phases d'évaluation correspondantes.

Ce document constitue la référence produit. Il ne prétend pas que toutes les décisions d'implémentation sont déjà prises.

---

# 17. Synthèse finale

ASF V2 doit remplacer le dashboard actuel par un espace de travail de développement conçu autour de l'orchestrateur.

Sa différence avec la V1 doit être visible dans l'usage quotidien :

- On ouvre directement un workspace.
- On travaille dans plusieurs projets.
- On discute avec l'orchestrateur.
- On délègue une mission.
- Les agents travaillent automatiquement.
- On suit l'avancement sans subir les détails techniques.
- On peut intervenir, éditer et déboguer.
- Le code et Git restent accessibles à tout moment.
- On retrouve son travail après fermeture.
- Les décisions du projet sont mémorisées.
- Les permissions et la publication restent contrôlées.

Le backend Rust existant représente le socle de fiabilité à préserver, tandis que les nouvelles capacités d'IDE, d'orchestration dynamique et de cycle de vie des sessions constituent les principaux investissements techniques.

**Définition de terminé :** ASF V2.0 est un produit complet, réellement utilisable et vérifié de bout en bout. Une refonte uniquement esthétique, un IDE incomplet ou une orchestration simulée ne satisfont pas cette spécification.
