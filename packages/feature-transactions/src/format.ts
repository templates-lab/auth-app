/** Presentation helpers shared by the transactions list and detail views. */

/**
 * Format a minor-units amount (e.g. cents) in its currency. Assumes the
 * currency's usual 2-decimal minor unit, which covers the codes this template
 * exercises; a production app would key the exponent off the currency.
 */
export function formatMoney(minorUnits: number, currency: string): string {
  try {
    return new Intl.NumberFormat(undefined, { style: "currency", currency }).format(
      minorUnits / 100,
    );
  } catch {
    // Unknown/invalid currency code: fall back to a plain amount plus the code.
    return `${(minorUnits / 100).toFixed(2)} ${currency}`;
  }
}

/** Turn a stored status token (`partially_refunded`) into a label. */
export function formatStatus(status: string): string {
  return status.replace(/_/g, " ");
}

/** Format a Unix epoch second count as a local date-time string. */
export function formatEpoch(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleString();
}
