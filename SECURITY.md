# Security Policy

Paschal handles material people intend to release only after they can no longer
act for themselves. We take security reports seriously.

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Instead, report
privately to **security@paschal.app** (replace with your own contact if you fork
this project). Include:

- a description of the issue and its impact,
- steps to reproduce, and
- any suggested remediation.

We aim to acknowledge reports within a few days and will keep you updated as we
investigate. Please give us reasonable time to release a fix before any public
disclosure. With your consent, we are happy to credit you for the discovery.

## Safe harbour for security research

We support good-faith security research and want researchers to feel safe
reporting issues to us. Australia does not provide a blanket statutory immunity
for security researchers, so this safe harbour works by **authorising** your
conduct: lawful, authorised access to and testing of a system does not amount to
an offence under the computer-offence provisions of the *Criminal Code Act 1995*
(Cth) (and the equivalent State and Territory laws), which turn on the access,
modification, or impairment being *unauthorised*.

If you make a good-faith effort to comply with this policy during your research,
we will:

- treat your activity as **authorised conduct** for the purposes of the
  *Criminal Code Act 1995* (Cth) and equivalent State and Territory computer-
  offence laws, so that it is not "unauthorised" access to, modification of, or
  impairment of data;
- not initiate or support any civil claim or criminal complaint against you in
  connection with research carried out consistent with this policy; and
- if a third party brings action against you for activity we authorised, make
  this authorisation known.

### Scope of this authorisation

Because Paschal is self-hosted software, this authorisation can only extend to
systems we actually control. It is limited to:

- the Paschal source code in this repository; and
- any test, demo, or reference instance that the maintainers explicitly operate
  and identify as in scope.

**We cannot authorise testing against an instance that someone else operates.** A
deployment you do not own or control belongs to its operator, and only that
operator can grant you safe harbour for it. Test against your own self-hosted
deployment, or against a maintainer-operated instance listed as in scope.

### Staying within safe harbour

To remain covered, you must:

- act in good faith and avoid privacy breaches, data loss, and degradation of
  service;
- only interact with accounts and content you own or have explicit permission to
  access. Paschal stores material intended for release after a person can no
  longer act for themselves, so **never access, copy, modify, retain, or disclose
  another person's sealed content**, and never attempt to recover or exfiltrate
  `kms.key`;
- stop immediately and report to us if you encounter real user data, credentials,
  or key material;
- comply with the *Privacy Act 1988* (Cth) and all other applicable Australian
  laws;
- keep the issue confidential and give us a reasonable period to investigate and
  remediate, coordinating the timing of any public disclosure with us.

### Out of scope

The following are not authorised and fall outside this safe harbour:

- denial-of-service (DoS/DDoS) or other availability or resource-exhaustion
  attacks;
- social engineering or phishing of maintainers, contributors, or users;
- physical attacks against people or property;
- automated mass-scanning that degrades service;
- accessing, testing, or attacking a deployment operated by a third party without
  that operator's consent; and
- accessing, exfiltrating, altering, or publishing user data.

This policy reflects the coordinated vulnerability disclosure practice
recommended by the Australian Signals Directorate's Australian Cyber Security
Centre (ASD's ACSC) and the vulnerability-disclosure guidance in the Australian
Government Information Security Manual (ISM).

## Operator responsibilities

Self-hosting means you own the security of your deployment. At a minimum:

- **Protect `kms.key`.** It is the encryption key for all sealed content. Store
  backups securely and separately from the server. Anyone with this key plus the
  database can decrypt vault contents.
- **Terminate TLS** in front of the application; never expose it over plain HTTP
  on the public internet.
- **Use a strong, unique `POSTGRES_PASSWORD`** and restrict network access to the
  database to the application container.
- **Keep the host and images patched**, and rebuild periodically to pick up
  dependency updates.
- **Publish your own vulnerability disclosure contact and safe-harbour terms** if
  you operate Paschal for others. The maintainers' safe harbour does not extend
  to your deployment; only you can authorise research against it.