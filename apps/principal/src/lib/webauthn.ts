/**
 * WebAuthn ceremony helpers and client-side ZK crypto.
 * All vault key material is derived and handled in this module — it never
 * leaves the browser in plaintext.
 */

import {
  startRegistration,
  startAuthentication,
  type RegistrationResponseJSON,
  type AuthenticationResponseJSON,
  type PublicKeyCredentialCreationOptionsJSON,
  type PublicKeyCredentialRequestOptionsJSON,
} from '@simplewebauthn/browser';

// ---------------------------------------------------------------------------
// Ceremony wrappers
// ---------------------------------------------------------------------------

export async function startPasskeyRegistration(
  optionsJSON: PublicKeyCredentialCreationOptionsJSON,
): Promise<RegistrationResponseJSON> {
  // Enable the PRF extension at creation time. On CTAP authenticators this
  // provisions the hmac-secret; many platform authenticators (iCloud Keychain,
  // Windows Hello) only return PRF results at assertion time if PRF was
  // requested here. Harmless for credentials that never back a Private Vault.
  // @simplewebauthn/browser passes `extensions` through to the WebAuthn call
  // verbatim, so an empty `prf: {}` is exactly the spec's "enable" signal.
  const withPrf: PublicKeyCredentialCreationOptionsJSON = {
    ...optionsJSON,
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    extensions: { ...(optionsJSON.extensions ?? {}), prf: {} } as any,
  };
  return startRegistration({ optionsJSON: withPrf });
}

export async function startPasskeyAuthentication(
  optionsJSON: PublicKeyCredentialRequestOptionsJSON,
): Promise<AuthenticationResponseJSON> {
  return startAuthentication({ optionsJSON });
}

// ---------------------------------------------------------------------------
// PRF-based vault key derivation
// ---------------------------------------------------------------------------

/**
 * Fixed, domain-separated salt for the WebAuthn PRF evaluation. The PRF output
 * is HKDF-expanded per vault (salt = vaultId) in `deriveVaultKeyFromPrf`, so a
 * single constant eval salt is correct: it yields one stable root secret per
 * credential, from which each vault gets a distinct sub-key. Only the
 * *constancy* of this value matters (its length is not significant) — change
 * it and every previously-sealed Private Vault becomes permanently unreadable.
 */
export const VAULT_PRF_SALT: Uint8Array = new TextEncoder().encode(
  'paschal:vault-key-derivation:v1',
);

/**
 * Returns a copy of `optionsJSON` with the PRF extension's `eval.first` set to
 * VAULT_PRF_SALT. This is the step that actually *requests* a PRF output —
 * without it the browser never evaluates the PRF and `clientExtensionResults`
 * carries no `prf` field.
 *
 * NB: @simplewebauthn/browser forwards `extensions` to
 * `navigator.credentials.get()` unchanged, so `first` must be a real
 * BufferSource here, *not* a base64url string, despite the JSON-shaped type.
 */
export function withPrfEval(
  optionsJSON: PublicKeyCredentialRequestOptionsJSON,
): PublicKeyCredentialRequestOptionsJSON {
  return {
    ...optionsJSON,
    extensions: {
      ...(optionsJSON.extensions ?? {}),
      prf: { eval: { first: VAULT_PRF_SALT } },
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
    } as any,
  };
}

/**
 * HKDF-expands a raw PRF output into a per-vault AES-GCM key-wrapping key.
 * Pure and deterministic given (prfOutput, vaultId) — the testable core of the
 * Zero-Knowledge derivation, independent of the WebAuthn ceremony.
 */
export async function deriveVaultKeyFromPrf(
  prfOutput: BufferSource,
  vaultId: string,
): Promise<CryptoKey> {
  const enc = new TextEncoder();
  const hkdfKey = await crypto.subtle.importKey('raw', prfOutput, 'HKDF', false, [
    'deriveKey',
  ]);
  return crypto.subtle.deriveKey(
    {
      name: 'HKDF',
      hash: 'SHA-256',
      salt: enc.encode(vaultId),
      info: enc.encode('vault-key-v1'),
    },
    hkdfKey,
    { name: 'AES-GCM', length: 256 },
    true,
    ['wrapKey', 'unwrapKey'],
  );
}

/**
 * Runs an authentication ceremony with the PRF extension to derive a
 * deterministic vault key from the authenticator hardware.
 *
 * Returns null if the authenticator does not return a PRF output (no PRF /
 * hmac-secret support). In that case the caller should show an error: "Your
 * passkey doesn't support client-side encryption. Use Standard tier or a
 * newer device."
 */
export async function deriveVaultKeyViaPrf(
  optionsJSON: PublicKeyCredentialRequestOptionsJSON,
  vaultId: string,
): Promise<{ key: CryptoKey; credential: AuthenticationResponseJSON } | null> {
  const res = await runPrfCeremony(optionsJSON);
  if (!res) return null;
  const key = await deriveVaultKeyFromPrf(res.prfOutput, vaultId);
  return { key, credential: res.credential };
}

/**
 * Runs a single PRF assertion and returns the raw PRF output. The caller can
 * expand it into both a per-vault key (`deriveVaultKeyFromPrf`) and the
 * recovery-key wrap key (`derivePrkWrapKeyFromPrf`) from one ceremony. Returns
 * null if the authenticator returns no PRF output.
 */
export async function runPrfCeremony(
  optionsJSON: PublicKeyCredentialRequestOptionsJSON,
): Promise<{ prfOutput: ArrayBuffer; credential: AuthenticationResponseJSON } | null> {
  const credential = await startAuthentication({
    optionsJSON: withPrfEval(optionsJSON),
    useBrowserAutofill: false,
  });
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const prfOutput: ArrayBuffer | undefined = (credential as any)
    .clientExtensionResults?.prf?.results?.first;
  if (!prfOutput) return null;
  return { prfOutput, credential };
}

// ---------------------------------------------------------------------------
// Principal-level Recovery Key (PRK)
// ---------------------------------------------------------------------------

/**
 * Domain-separated HKDF expansion of the PRF output into the key that wraps the
 * principal's Recovery Key. Distinct from the per-vault derivation (different
 * salt+info), so the PRK-wrap key is never equal to any vault key.
 */
export async function derivePrkWrapKeyFromPrf(prfOutput: BufferSource): Promise<CryptoKey> {
  const enc = new TextEncoder();
  const hkdfKey = await crypto.subtle.importKey('raw', prfOutput, 'HKDF', false, ['deriveKey']);
  return crypto.subtle.deriveKey(
    {
      name: 'HKDF',
      hash: 'SHA-256',
      salt: enc.encode('paschal:prk-wrap'),
      info: enc.encode('prk-wrap-v1'),
    },
    hkdfKey,
    { name: 'AES-GCM', length: 256 },
    true,
    ['wrapKey', 'unwrapKey'],
  );
}

/** Generate a fresh principal Recovery Key (it wraps vault DEKs, so it needs
 *  the wrap usages). Extractable so it can itself be wrapped for storage. */
export async function generatePrk(): Promise<CryptoKey> {
  return crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, true, [
    'wrapKey',
    'unwrapKey',
  ]);
}

/** AES-GCM key-wrap of one key under another. Generic over what's wrapped
 *  (a PRK under a KEK, or a DEK under a PRK). */
export async function wrapAesKey(
  wrappingKey: CryptoKey,
  key: CryptoKey,
): Promise<{ ciphertext: Uint8Array; nonce: Uint8Array }> {
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const wrapped = await crypto.subtle.wrapKey('raw', key, wrappingKey, {
    name: 'AES-GCM',
    iv: nonce,
  });
  return { ciphertext: new Uint8Array(wrapped), nonce };
}

/** Unwrap a PRK (which itself wraps DEKs — hence the wrap usages). */
export async function unwrapPrk(
  wrappingKey: CryptoKey,
  ciphertext: Uint8Array,
  nonce: Uint8Array,
): Promise<CryptoKey> {
  return crypto.subtle.unwrapKey(
    'raw',
    new Uint8Array(ciphertext),
    wrappingKey,
    { name: 'AES-GCM', iv: new Uint8Array(nonce) },
    { name: 'AES-GCM', length: 256 },
    true,
    ['wrapKey', 'unwrapKey'],
  );
}

// ---------------------------------------------------------------------------
// Vault DEK management (AES-256-GCM via Web Crypto)
// ---------------------------------------------------------------------------

export async function generateVaultDek(): Promise<CryptoKey> {
  return crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, true, [
    'encrypt',
    'decrypt',
  ]);
}

export async function wrapVaultDek(
  vaultKey: CryptoKey,
  dek: CryptoKey,
): Promise<{ ciphertext: Uint8Array; nonce: Uint8Array }> {
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const wrapped = await crypto.subtle.wrapKey('raw', dek, vaultKey, {
    name: 'AES-GCM',
    iv: nonce,
  });
  return { ciphertext: new Uint8Array(wrapped), nonce };
}

export async function unwrapVaultDek(
  vaultKey: CryptoKey,
  ciphertext: Uint8Array,
  nonce: Uint8Array,
): Promise<CryptoKey> {
  return crypto.subtle.unwrapKey(
    'raw',
    new Uint8Array(ciphertext),
    vaultKey,
    { name: 'AES-GCM', iv: new Uint8Array(nonce) },
    { name: 'AES-GCM', length: 256 },
    true,
    ['encrypt', 'decrypt'],
  );
}

// ---------------------------------------------------------------------------
// Letter encryption
// ---------------------------------------------------------------------------

/**
 * Domain-separated Additional Authenticated Data binding a Private letter body
 * to its vault. Passed as GCM AAD so a ciphertext sealed for one vault cannot be
 * moved into another vault's letter slot without the auth tag failing
 * (ZK crypto audit M1). The vault id is known at both seal and read-back.
 */
export function letterAad(vaultId: string): Uint8Array {
  // ASCII-only (constant prefix + UUID), encoded into a fresh ArrayBuffer-backed
  // array so it satisfies BufferSource for SubtleCrypto's additionalData.
  const s = `paschal:zk-letter:v2:${vaultId}`;
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
  return out;
}

export async function encryptLetter(
  dek: CryptoKey,
  plaintext: string,
  aad?: Uint8Array,
): Promise<{ ciphertext: Uint8Array; nonce: Uint8Array }> {
  const nonce = crypto.getRandomValues(new Uint8Array(12));
  const data = new TextEncoder().encode(plaintext);
  const params: AesGcmParams = { name: 'AES-GCM', iv: nonce };
  if (aad) params.additionalData = new Uint8Array(aad);
  const encrypted = await crypto.subtle.encrypt(params, dek, data);
  return { ciphertext: new Uint8Array(encrypted), nonce };
}

export async function decryptLetter(
  dek: CryptoKey,
  ciphertext: Uint8Array,
  nonce: Uint8Array,
  aad?: Uint8Array,
): Promise<string> {
  const params: AesGcmParams = { name: 'AES-GCM', iv: new Uint8Array(nonce) };
  if (aad) params.additionalData = new Uint8Array(aad);
  const decrypted = await crypto.subtle.decrypt(params, dek, new Uint8Array(ciphertext));
  return new TextDecoder().decode(decrypted);
}

// ---------------------------------------------------------------------------
// Recovery phrase (PBKDF2)
// ---------------------------------------------------------------------------

export function generateRecoveryPhrase(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

/**
 * A strong, transcribable passphrase for the manual heir mode (~79 bits): 16
 * chars from a 31-symbol unambiguous alphabet, dash-grouped. Rejection-sampled
 * to avoid modulo bias. Generating one by default stops users picking a weak,
 * brute-forceable passphrase (ZK crypto audit M3).
 */
export function generateStrongPassphrase(): string {
  const alphabet = 'abcdefghjkmnpqrstuvwxyz23456789'; // no ambiguous l/i/o/0/1
  const len = 16;
  const max = 256 - (256 % alphabet.length); // reject bytes >= 248 to debias
  const chars: string[] = [];
  while (chars.length < len) {
    const b = crypto.getRandomValues(new Uint8Array(1))[0] ?? 0;
    if (b < max) chars.push(alphabet[b % alphabet.length] ?? 'a');
  }
  // Group into 4s for readability: "k7m2-pq9r-..." (16 chars ≈ 79 bits).
  return chars.join('').replace(/(.{4})(?=.)/g, '$1-');
}

/**
 * Derives a 256-bit AES-GCM key from a recovery phrase using PBKDF2.
 * 600,000 iterations — slow by design.
 */
export async function deriveRecoveryKey(
  phrase: string,
  salt: Uint8Array,
): Promise<CryptoKey> {
  const keyMaterial = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(phrase),
    'PBKDF2',
    false,
    ['deriveKey'],
  );
  return crypto.subtle.deriveKey(
    {
      name: 'PBKDF2',
      salt: new Uint8Array(salt),
      iterations: 600_000,
      hash: 'SHA-256',
    },
    keyMaterial,
    { name: 'AES-GCM', length: 256 },
    true,
    ['wrapKey', 'unwrapKey'],
  );
}

// ---------------------------------------------------------------------------
// Raw-key helpers (auto heir modes)
// ---------------------------------------------------------------------------

/** Import 32 raw bytes as an AES-GCM key (for sealing a letter body directly). */
export async function importAesGcmKey(
  raw: Uint8Array,
  usages: KeyUsage[],
): Promise<CryptoKey> {
  return crypto.subtle.importKey('raw', new Uint8Array(raw), { name: 'AES-GCM' }, false, usages);
}

/** Byte-wise XOR (for splitting/recombining a key into two shares). */
export function xorBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length);
  for (let i = 0; i < a.length; i++) out[i] = (a[i] ?? 0) ^ (b[i] ?? 0);
  return out;
}

// ---------------------------------------------------------------------------
// Heir passphrase (recipient release path)
// ---------------------------------------------------------------------------

/**
 * Derive an AES-GCM content key from a recipient passphrase (PBKDF2, 600k
 * iterations). Unlike `deriveRecoveryKey` (which wraps keys), this key seals
 * the letter body directly — so its usages are encrypt/decrypt. The heir claim
 * page derives the identical key to open the letter.
 */
export async function deriveHeirKey(
  passphrase: string,
  salt: Uint8Array,
): Promise<CryptoKey> {
  const keyMaterial = await crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(passphrase),
    'PBKDF2',
    false,
    ['deriveKey'],
  );
  return crypto.subtle.deriveKey(
    {
      name: 'PBKDF2',
      salt: new Uint8Array(salt),
      iterations: 600_000,
      hash: 'SHA-256',
    },
    keyMaterial,
    { name: 'AES-GCM', length: 256 },
    true,
    ['encrypt', 'decrypt'],
  );
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

export function toBase64Url(bytes: Uint8Array): string {
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=/g, '');
}

export function fromBase64Url(s: string): Uint8Array {
  const padded = s.replace(/-/g, '+').replace(/_/g, '/');
  const pad = (4 - (padded.length % 4)) % 4;
  const binary = atob(padded + '='.repeat(pad));
  return Uint8Array.from(binary, (c) => c.charCodeAt(0));
}

export function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < hex.length; i += 2) {
    bytes[i / 2] = parseInt(hex.slice(i, i + 2), 16);
  }
  return bytes;
}

export function bytesToHex(bytes: Uint8Array): string {
  let hex = '';
  for (const b of bytes) hex += b.toString(16).padStart(2, '0');
  return hex;
}
