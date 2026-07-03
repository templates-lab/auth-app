/** Trim and lower-case an email for display/comparison. */
export function normalizeEmail(email: string): string {
  return email.trim().toLowerCase();
}
