/** Integer minor units only — never floats. UGX has exponent 0. */
export function formatMoney(minor: number, currency = 'UGX'): string {
  return new Intl.NumberFormat('en-UG', {
    style: 'currency',
    currency,
    currencyDisplay: 'code',
    maximumFractionDigits: 0
  }).format(minor);
}

/** Round a tender total to the cash step, half away from zero. */
export function roundToStep(minor: number, step: number): number {
  if (step <= 0) return minor;
  return Math.round(minor / step) * step;
}

/** basis points -> integer minor units, rounded half away from zero. */
export function discountMinor(minor: number, bp: number): number {
  return Math.round((minor * bp) / 10000);
}
