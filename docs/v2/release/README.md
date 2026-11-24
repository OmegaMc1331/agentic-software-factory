# ASF V2 release-candidate qualification

ASF V2.0 remains **NO-GO**. The source is an independently prepared snapshot on
the public V1 lineage. It preserves the V1 entry point and the V2 implementation.
Historical qualification receipts and their source mappings remain in private
custody. Published source, build inputs, test implementations and synthetic
fixtures are available to repeat qualification.

Use the [reproduction procedure](../../../scripts/v2_p14/README.md), the
[native matrix](SUPPORTED-PLATFORMS.md) and the [mandatory release checklist](RELEASE-CHECKLIST.md).
See the [installation guide](INSTALLATION.md), [upgrade guide](UPGRADE.md),
[security model](SECURITY.md) and [known issues](KNOWN-ISSUES.md) before operating
a candidate. Build and attach evidence to the exact candidate commit and artifacts.

## Evidence custody and publication review

Store raw qualification output in `.factory/private-evidence/` or an external
private evidence location under your control. Keep execution logs, session
records, user-profile locations and original captures there. Public summaries
can include reproducible commands, aggregate results and reviewed derivatives.
The V1 inventory under `scripts/fixtures/` contains the public compatibility
reference. It is required by the test harness.

Read the actual public refs before screening. Run the following from the
isolated publication checkout, supplying the verified full Git object IDs
for every advertised ref and the approved public commit email.

```text
python -B scripts/v2_p14/publication_audit.py CANDIDATE --public-ref PUBLIC_MAIN --public-ref OTHER_PUBLIC_REF --public-email PUBLIC_EMAIL --output .factory/publication-audit.json
```

Record only redacted findings and immutable object identifiers. Review flagged
paths with synthetic values against their exact blob and line. Inspect every
included image and non-text artifact, including rendered text and metadata.
Exclude any artifact whose content cannot be reviewed. Run an independent
secret scan over outgoing history and the candidate tree. Verify that the local
candidate has only public ancestry and compare its build and test inputs with
the intended implementation before seeking publication authorization.
