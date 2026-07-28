# Public Security Disclosure

The public repository must enable GitHub private vulnerability reporting before preview. Sensitive reports must not be routed through public issues.

The canonical process is documented in [`SECURITY.md`](../../SECURITY.md). A public release also requires:

- a reviewed threat model;
- dependency, secret, binary, SBOM, and license evidence from the exact source commit;
- an archive-aware and all-ref history scan;
- confirmation that the public history contains no recovery transport fragments or unwanted identity metadata;
- proof that V1 contains no signer, credential, custody, or order-placement path;
- independent review and a coordinated-disclosure owner.

This document defines a release requirement. It is not evidence that the security review or public reporting feature is already complete.
