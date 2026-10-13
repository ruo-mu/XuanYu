<p align="center">
  <img src="../apps/desktop-ui/public/assets/xuanyu.png" alt="XuanYu Agent" width="180" />
</p>

<h1 align="center">XuanYu Agent · 玄语</h1>

<p align="center">
  <strong>Espace de travail d'agent IA de bureau local-first</strong><br/>
  <sub>Le raisonnement et l'orchestration tournent dans un moteur Rust dédié, la mémoire et les données restent sur votre machine</sub>
</p>

<p align="center">
  <a href="../README.md"><img alt="简体中文" src="https://img.shields.io/badge/简体中文-d9d9d9"></a>
  <a href="./README.en.md"><img alt="English" src="https://img.shields.io/badge/English-d9d9d9"></a>
  <a href="./README.ja.md"><img alt="日本語" src="https://img.shields.io/badge/日本語-d9d9d9"></a>
  <a href="./README.es.md"><img alt="Español" src="https://img.shields.io/badge/Español-d9d9d9"></a>
  <a href="./README.fr.md"><img alt="Français" src="https://img.shields.io/badge/Français-d9d9d9"></a>
</p>

<p align="center">
  <a href="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue"><img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue" alt="Platform"/></a>
  <a href="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9"><img src="https://img.shields.io/badge/version-0.1.0--preview-0ea5e9" alt="Version"/></a>
  <a href="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6"><img src="https://img.shields.io/badge/tech-Electron%20%2B%20Rust%20%2B%20PostgreSQL-8b5cf6" alt="Stack technique"/></a>
  <a href="https://img.shields.io/badge/engine-15%20crates-16a34a"><img src="https://img.shields.io/badge/engine-15%20crates-16a34a" alt="Cargo Workspace"/></a>
  <a href="./LICENSE"><img src="https://img.shields.io/badge/license-MIT-green" alt="Licence"/></a>
</p>

---

**玄语 (XuanYu Agent)** est un espace de travail d'agent IA de bureau **local-first**, avec prise en charge native de Windows, macOS et Linux. Le raisonnement, l'exécution des outils et la gouvernance de sécurité s'exécutent dans un processus natif Rust dédié ; Electron ne porte que l'interface et les interactions avec l'appareil. Sessions, mémoire à long terme et journaux d'exécution résident intégralement dans le **PostgreSQL fourni avec l'application** (pgvector inclus) — utilisable hors ligne, les données ne quittent pas la machine, tout en pouvant se connecter à n'importe quel modèle de premier plan et à des capacités tierces.

> [!NOTE]
> Version d'aperçu `0.1.0-preview` actuelle, prise en charge native : **Windows 10+, macOS (x64 / arm64) et Linux (x64)**.

## Table des matières

- [Fonctionnalités](#fonctionnalités)
- [Aperçu de l'architecture](#aperçu-de-larchitecture)
- [Structure du workspace](#structure-du-workspace)
- [Fonctionnement interne](#fonctionnement-interne)
- [Démarrage rapide](#démarrage-rapide)
- [Construire un paquet de distribution](#construire-un-paquet-de-distribution)
- [Prérequis](#prérequis)
- [Configuration](#configuration)
- [Modèle de sécurité](#modèle-de-sécurité)
- [Feuille de route](#feuille-de-route)
- [Communauté et contribution](#communauté-et-contribution)
- [Licence](#licence)

## Fonctionnalités

- **Moteur natif local** — Le raisonnement et l'orchestration s'exécutent dans un processus dédié `xuanyu-engine-server`, totalement découplé de l'UI. Un crash du moteur est intercepté par la coque qui le redémarre automatiquement ; il peut aussi être réutilisé sans interface (CLI / service headless).
- **Mémoire à long terme** — Recherche sémantique PostgreSQL + pgvector en premier lieu, complétée par une recherche plein texte et un index local Tantivy pour une récupération multi-voies, assurant la mémoire inter-sessions et la sédimentation des faits clés.
- **Marketplace de plugins / MCP / skills** — Parcourez et installez plugins, services MCP et skills depuis l'application pour étendre les capacités de l'Agent à la demande, sans toucher au code.
- **Connexion multi-modèles** — Un assistant de configuration guide l'intégration des principaux fournisseurs, avec routage par tâche (titre / planification / implémentation / revue, etc.) vers différents modèles, pour concilier coût et qualité.
- **Collaboration multi-Agent** — Décomposition des tâches en sous-Agents, combinée à un flux « revue - fusion » pour mener à bien des tâches d'ingénierie complexes.
- **Intégration multicanal** — WeChat, Feishu, DingTalk, Telegram, Discord, QQ, Line, WhatsApp et autres messageries externes convergent vers un même ensemble de sessions et de mémoire.
- **Gouvernance de sécurité** — Liste noire de commandes, gradation des commandes git destructrices, validation de portée des chemins, bac à sable de l'OS : toute opération à risque exige une approbation humaine.
- **Réédition réversible** — Lorsque vous modifiez et renvoyez n'importe quel message utilisateur de l'historique, seules les modifications de fichiers produites par l'IA lors de ce tour sont annulées précisément (instantanés par fichier + restauration différentielle), préservant votre travail non validé sans rapport.
- **Optimisations de performance** — Exécution spéculative masquant la latence des allers-retours d'outils, compression automatique du contexte, reprise après troncature : les tâches longues ne s'interrompent pas.
- **Expérience de bureau moderne** — Electron + antd, thèmes clair / sombre, rendu Markdown en streaming, coloration syntaxique et vue Diff intégrés.

## Aperçu de l'architecture

```text
Couche de rendu (React / Vite / antd)
        │  IPC / JSON-RPC
        ▼
Moteur Rust (xuanyu-engine-server)
        │  SQL
        ▼
PostgreSQL + pgvector
```

Trois axes de conception :

- **Découplage des processus** — Le moteur est un processus indépendant qui dialogue en bidirectionnel avec la coque de bureau via JSON-RPC sur stdin/stdout (voir [engine_server.rs](../crates/xuanyu-cli/src/bin/engine_server.rs)). Il ne dépend pas de l'interface : il peut s'exécuter et se tester isolément.
- **Piloté par événements** — Toute variation asynchrone dans le moteur (incréments de Turn, progression des outils, demandes d'autorisation, mises à jour de mémoire, etc.) passe par `eventbus`, sérialisée en `AppEvent` et poussée activement vers l'UI. L'UI ne fait aucun polling et prend naturellement en charge l'abonnement multi-clients.
- **Stockage à source unique de vérité** — Sessions, messages, mémoire, journaux d'exécution et journaux d'événements résident tous dans le PostgreSQL embarqué, avec un schéma unique.

## Structure du workspace

```text
apps/
  desktop/            Coque Electron (processus principal / preload / configuration de packaging)
  desktop-ui/         Couche de rendu (React + Vite + antd)
crates/               15 crates Rust (Cargo workspace)
proto/                Définition du protocole entre le moteur et la coque (app.proto / session.proto)
scripts/              Génération de code IPC, packaging et scripts de smoke test
```

Aperçu des crates côté moteur :

| Crate | Rôle |
|-------|------|
| `xuanyu-core` | Couche d'intégration du moteur : AppCore, sessions / mémoire / marketplace / canaux |
| `xuanyu-execution` | Runtime d'exécution : boucle principale / boucle d'outils / assemblage du contexte / budget de tokens |
| `xuanyu-session` | Domaine des sessions : SessionManager / threads / isolation / file de messages |
| `xuanyu-memory` | Mémoire à long terme : recherche sémantique pgvector + FTS + Tantivy + embedding |
| `xuanyu-tools` | Runtime d'outils : MCP / MCP distant / bac à sable OS / délimitation des sorties |
| `xuanyu-governance` | Gouvernance de sécurité : validation des commandes / gradation git / validation des chemins / rollback / comptabilité |
| `xuanyu-channels` | Canaux externes : WeChat / Feishu / DingTalk / Telegram / Discord, etc. |
| `xuanyu-eventbus` | Bus d'événements : spine / router / transformer / backpressure |
| `xuanyu-storage` | Stockage et plan de contrôle : pool de connexions / tables de session / PostgreSQL embarqué |
| `xuanyu-protocol` | Contrats de protocole : Envelope / AppEvent / AppError / DTO |
| `xuanyu-cli` | CLI et engine-server |
| `xuanyu-desktop-host` | Pont hôte de bureau : expose des commandes IPC transcrites automatiquement |
| `xuanyu-agent-bridge` | Pont Agent : liaison d'espace de travail des sous-Agents |
| `xuanyu-agents` | Runtime des sous-Agents : watchdog / assess_merge |
| `xuanyu-extensions` | Extensions de capacités : chargement des plugins / MCP / skills |

## Fonctionnement interne

### Cycle de vie d'une session

Un message utilisateur entre dans [AppCore](../crates/xuanyu-core/src/lib.rs) puis franchit quatre étapes :

1. **Assemblage du contexte** — [prepare_turn](../crates/xuanyu-execution/src/context_assembler.rs) décide, selon `loadMode` (lazy / balanced / eager), des contenus à injecter ce tour-ci : historique, résultats de recherche en mémoire, faits clés, etc.
2. **Boucle principale** — [run_main_turn_loop](../crates/xuanyu-execution/src/main_turn_loop.rs) appelle le modèle et traite les appels d'outils de manière répétée dans la limite d'itérations, avec prise en charge d'une échéance en temps mural `deadline_ms` et d'un plafond souple d'itérations.
3. **Distribution des outils** — Détecte les appels d'outils natifs ou « en mode impression » (compatible avec les modèles ne prenant pas nativement en charge les tool calls), les confie à `xuanyu-tools` pour exécution, puis réinjecte les résultats dans la transcription pour le tour suivant.
4. **Conclusion** — Lorsque le modèle ne demande plus d'outil, le tour se termine avec `final_text`.

Points d'ingénierie clés :

- **Compression du contexte** — Les constantes de budget figurent dans [session_context.rs](../crates/xuanyu-execution/src/session_context.rs) ; elle est déclenchée par la configuration `contextManagement.autoCompactThreshold`, élague les anciens résultats d'outils et ajoute un handoff summary, évitant les ruptures d'information.
- **Reprise après troncature** — Lorsque `finish_reason` vaut `length` / `max_tokens`, la réponse n'est pas considérée comme finale : une invite de continuation est injectée pour le tour suivant.
- **Exécution spéculative** — [speculative_tools.rs](../crates/xuanyu-core/src/speculative_tools.rs) met en file d'exécution dès que les paramètres d'un appel sont scellés dans le flux, et récupère directement le résultat au moment du dispatch, masquant la latence des allers-retours d'outils.

### Mémoire à long terme

Portée par [xuanyu-memory](../crates/xuanyu-memory/src), une pile mémoire multi-backends et multi-voies de récupération :

- `embedding` prend en charge trois sources : API compatible OpenAI, inférence locale, hachage hors ligne (par défaut, prêt à l'emploi même sans clé API).
- La recherche vectorielle pgvector est privilégiée (Top-N par distance cosinus) ; après ≥ 3 échecs consécutifs, un disjoncteur s'enclenche automatiquement et rétrograde vers la recherche par mots-clés PostgreSQL FTS.
- L'index local Tantivy ([tantivy_store.rs](../crates/xuanyu-memory/src/tantivy_store.rs)) sert de récupération de secours en fonctionnement totalement hors ligne.

### Communication front-end / moteur

Les contrats de protocole sont définis dans [xuanyu-protocol](../crates/xuanyu-protocol/src/lib.rs) : une enveloppe générique `Envelope<T>` porte `correlation_id` / `trace_id` pour la corrélation des requêtes et le suivi de chaîne, et un `AppError` unifié (avec error code / recoverable / suggestion) remplace les codes d'erreur bruts.

Le pont IPC entre le front-end et le moteur est généré automatiquement par script, afin d'éviter toute dérive de contrat due à une écriture manuelle :

| Script | Rôle |
|------|------|
| [gen-desktop-dispatch.py](../scripts/gen-desktop-dispatch.py) | Analyse la signature des fonctions `xuanyu-desktop-host`, génère le wrapper de dispatch |
| [gen-desktop-ipc-wrappers.py](../scripts/gen-desktop-ipc-wrappers.py) | Génère les wrappers d'appel TypeScript |
| [gen-ipc-command-map.py](../scripts/gen-ipc-command-map.py) | Vérifie la couverture des commandes des deux côtés |

Après avoir modifié les capacités du moteur, exécutez `pnpm gen:desktop-ipc` pour aligner les contrats front-end / back-end.

## Démarrage rapide

### Utilisation directe (recommandée)

Rendez-vous dans [Releases](https://github.com/ruo-mu/XuanYu/releases) pour télécharger le paquet correspondant à votre plateforme ; il fonctionne dès l'installation. Au premier démarrage, l'assistant de configuration s'ouvre : choisissez un modèle et vous pouvez commencer.

| Plateforme | Installateur | Portable |
|------|------|------|
| Windows | `nsis` | `zip` |
| macOS | `dmg` | `zip` |
| Linux | `deb` | `AppImage`, `tar.gz` |

Les artefacts suivent la nomenclature `XuanYu-Agent-<version>-<os>[-<arch>].<ext>`, par exemple `XuanYu-Agent-0.1.0-preview-windows-setup.exe`, `XuanYu-Agent-0.1.0-preview-mac-arm64.dmg` ou `XuanYu-Agent-0.1.0-preview-linux.AppImage` ; macOS porte `arch` en raison de ses deux architectures, les autres plateformes pouvant l'omettre. Les versions portables utilisent les mêmes répertoires standard de la plateforme que l'installateur et n'écrivent jamais de données dans le répertoire d'installation.

PostgreSQL + pgvector sont intégrés à l'artefact : récupérés au moment du build, sans téléchargement ni décompression à l'installation ou à l'exécution. Les paramètres de connexion sont fixés par l'application : aucune chaîne de connexion, aucun hôte, port ou identifiant à configurer, et le premier démarrage ne nécessite pas de réseau.

Les paquets ne sont ni signés ni notariés lors de cette étape : sur macOS, Gatekeeper bloque le premier lancement (clic droit pour ouvrir et autoriser), et sur Windows un avertissement SmartScreen « éditeur inconnu » s'affiche.

### Développement depuis les sources

Prérequis : Rust stable, Node.js 22+, pnpm.

```bash
pnpm install
pnpm dev          # Démarre Electron et le moteur local
```

Autres commandes utiles :

```bash
pnpm typecheck    # Vérification de types du processus principal + couche de rendu
pnpm smoke        # Tests de fumée
pnpm gen:desktop-ipc   # Régénère le contrat IPC après modification des capacités du moteur
```

## Construire un paquet de distribution

```bash
pnpm dist
```

Cette commande enchaîne : compilation du moteur en mode release → build du front-end → empaquetage des artefacts de distribution pour la plateforme courante (Windows `nsis` / `zip`, macOS `dmg` / `zip`, Linux `AppImage` / `deb` / `tar.gz`). Les artefacts se trouvent dans `apps/desktop/release-dist/`.

> [!IMPORTANT]
> Réexécuter `pnpm install` écrase le patch du template NSIS d'electron-builder. Le dépôt configure un hook `postinstall` qui le réapplique automatiquement ; en cas d'anomalie de l'installateur, vérifiez d'abord que le patch est bien appliqué.

## Prérequis

| Catégorie | Exigence |
|------|------|
| Système d'exploitation | Windows 10+, macOS (x64 / arm64), Linux (x64) |
| Runtime | PostgreSQL 15+ et pgvector intégrés à l'artefact ; rien à installer soi-même |
| Développement | Node.js 22+, pnpm 10+, Rust stable |

## Configuration

La configuration, les données, le cache et les journaux suivent les répertoires standard de chaque plateforme :

| Catégorie | Windows | macOS | Linux |
|------|---------|-------|-------|
| Configuration | `%APPDATA%\XuanYu\` | `~/Library/Application Support/XuanYu/` | `$XDG_CONFIG_HOME/xuanyu/` (par défaut `~/.config/xuanyu/`) |
| Données | `%LOCALAPPDATA%\XuanYu\` | `~/Library/Application Support/XuanYu/` | `$XDG_DATA_HOME/xuanyu/` (par défaut `~/.local/share/xuanyu/`) |
| Cache | `%LOCALAPPDATA%\XuanYu\Cache\` | `~/Library/Caches/XuanYu/` | `$XDG_CACHE_HOME/xuanyu/` (par défaut `~/.cache/xuanyu/`) |
| Journaux | `%LOCALAPPDATA%\XuanYu\logs\` | `~/Library/Logs/XuanYu/` | `$XDG_STATE_HOME/xuanyu/logs/` (par défaut `~/.local/state/xuanyu/logs/`) |

L'ancien ancrage `~/.xuanyu` est abandonné.

L'assistant de configuration est accessible par deux entrées : le **menu « Fichier »** de la barre de titre, ou le **bouton « Assistant de configuration »** de la page de paramètres système — vous pouvez y revenir à tout moment après la configuration initiale. La connexion à la base utilise des paramètres fixes intégrés à l'application et n'expose donc plus de chaîne de connexion, d'hôte, de port ni d'identifiants. La logique de validation de la configuration figure dans [config_schema.rs](../crates/xuanyu-core/src/config_schema.rs).

## Modèle de sécurité

- **Liste noire de commandes** — Intercepte les commandes dangereuses telles que `rm -rf /`, `sudo rm`, `chmod 777`, `eval` ([command_validator.rs](../crates/xuanyu-governance/src/command_validator.rs)).
- **Gradation git** — Les commandes destructrices (`rebase` / `reset` / `clean` / `filter-*`, etc.) sont systématiquement refusées ([git_command_policy.rs](../crates/xuanyu-governance/src/git_command_policy.rs)).
- **Validation des chemins** — Les opérations sur fichiers sont restreintes à la portée isolée de la session ([path_validator.rs](../crates/xuanyu-governance/src/path_validator.rs)).
- **Bac à sable OS** — Encapsulation des commandes via `sandbox-exec` sur macOS et `bwrap` sur Linux ; Windows ne dispose pas d'encapsuleur de processus équivalent, cet interrupteur ne s'y applique donc pas ([os_sandbox.rs](../crates/xuanyu-tools/src/os_sandbox.rs)).
- **Approbation humaine** — Un appel d'outil à risque déclenche `PermissionRequested` ; après validation dans une boîte de dialogue, le flux revient via `PermissionAnswered`, avec traçabilité complète ([turn_ledger.rs](../crates/xuanyu-governance/src/turn_ledger.rs)).
- **Sécurité du rollback** — La restauration de l'espace de travail s'effectue par restauration différentielle d'instantanés de contenu et **n'utilise jamais** de commandes destructrices telles que `git reset --hard` ; les fichiers modifiés manuellement par l'utilisateur sont exclus de la restauration et listés comme conflits.

Pour signaler un problème de sécurité, voir [SECURITY.md](../.github/SECURITY.md).

## Feuille de route

- [ ] Sessions en temps réel et abonnement piloté par événements
- [ ] Distribution en ligne et mise à jour automatique du marketplace d'Agents / skills
- [ ] Modèles de politiques de revue et de permissions plus riches
- [ ] Optimisation du démarrage à froid et des performances
- [ ] Visualisation de la mémoire et re-ranking de la recherche hybride

## Communauté et contribution

- [CONTRIBUTING.md](../.github/CONTRIBUTING.md) — Normes et processus de contribution
- [CODE_OF_CONDUCT.md](../.github/CODE_OF_CONDUCT.md) — Code de conduite de la communauté
- [SECURITY.md](../.github/SECURITY.md) — Comment signaler un problème de sécurité
- [Modèles d'Issue](../.github/ISSUE_TEMPLATE) — Retours de bugs et suggestions de fonctionnalités

Vous avez trouvé un bug ou une idée d'amélioration ? N'hésitez pas à ouvrir une Issue ou une Pull Request.

## Licence

Ce projet est distribué sous [MIT License](../LICENSE).