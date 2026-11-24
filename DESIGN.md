# ASF design context

The approved [product specification](docs/v2/product-spec.md) is normative. The implemented [design system](docs/v2/design-system.md), [component contracts](docs/v2/component-inventory.md) and [workspace visual specification](docs/v2/workspace-visual-spec.md) describe the P02 foundation and P04 production integration. [PRODUCT.md](PRODUCT.md) records the supplied product context.

Build professional developer tooling: flat surfaces, thin separators, compact actions, readable Inter and JetBrains Mono, restrained blue focus/selection, contextual detail and an orchestrator-centered workspace. Avoid marketing heroes, decorative cards/metrics, gradients, robot avatars and invented activity.

Use `apps/dashboard/src/v2/design-system/tokens`: palette → semantic theme → component aliases. Components consume semantic properties, never new raw palette values. Light/dark must include code, diff, output and overlays. Respect reduced motion and explicit keyboard focus. Use real native controls and typed domain records with provenance.

P02 is isolated at `v2.html`; V1 production navigation stays intact. Prototype actions operate on labeled browser fixtures only. P03/P04 production routes use Rust as authority for projects, processes, policies and persistence. The real opt-in workspace is `/workspace.html`; its API data is distinct from P02 fixtures. Files, LSP/DAP, conversation and later services retain their planned phase boundaries. Do not treat this prototype as permission to start later phases or a production backend.
