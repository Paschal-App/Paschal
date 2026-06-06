// Session storage. localStorage is enough for the MVP; production swaps to
// httpOnly cookies (see specs/13).

import { browser } from '$app/environment';

const TOKEN_KEY = 'paschal.session_token';
const EMAIL_KEY = 'paschal.email';
const PRINCIPAL_KEY = 'paschal.principal_id';

export interface Session {
  token: string;
  email: string;
  principalId: string;
}

export function load(): Session | null {
  if (!browser) return null;
  const token = localStorage.getItem(TOKEN_KEY);
  const email = localStorage.getItem(EMAIL_KEY);
  const principalId = localStorage.getItem(PRINCIPAL_KEY);
  if (!token || !email || !principalId) return null;
  return { token, email, principalId };
}

export function save(session: Session): void {
  if (!browser) return;
  localStorage.setItem(TOKEN_KEY, session.token);
  localStorage.setItem(EMAIL_KEY, session.email);
  localStorage.setItem(PRINCIPAL_KEY, session.principalId);
}

export function clear(): void {
  if (!browser) return;
  localStorage.removeItem(TOKEN_KEY);
  localStorage.removeItem(EMAIL_KEY);
  localStorage.removeItem(PRINCIPAL_KEY);
}
