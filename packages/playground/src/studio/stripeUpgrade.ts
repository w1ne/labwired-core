// Stripe Payment Links for LabWired paid tiers. The Payment Links themselves
// are created in the Stripe dashboard; this module appends Stripe's documented
// query-string overrides so the webhook can map the resulting workspace back
// to a Clerk user.
//
// Stripe supports `client_reference_id` and `prefilled_email` on Payment Link
// URLs — both flow through to the resulting `checkout.session.completed`
// event without any API call.
//   https://stripe.com/docs/payment-links/url-parameters

export const STRIPE_PRO_PAYMENT_LINK = 'https://buy.stripe.com/bJeaEW56u3H16Tc3Gz5AQ03';

// TODO(billing): replace `REPLACE_DESIGNER_LINK` with the real Designer
// Payment Link after creating the "LabWired Designer" $5/seat/mo product +
// Payment Link in Stripe Dashboard. Until that's done, the Designer CTA will
// point at a 404 — keep the marketing card hidden behind a feature flag if
// you ship before the Stripe side is set up.
export const STRIPE_DESIGNER_PAYMENT_LINK = 'https://buy.stripe.com/REPLACE_DESIGNER_LINK';

export type StripeUpgradeTier = 'designer' | 'pro';

export interface StripeUpgradeContext {
  /** Which paid tier the Payment Link should target. Defaults to 'pro' for backward compat. */
  tier?: StripeUpgradeTier;
  clerkUserId?: string | null;
  email?: string | null;
}

export function buildStripeUpgradeUrl(ctx: StripeUpgradeContext = {}): string {
  const tier = ctx.tier ?? 'pro';
  const base = tier === 'designer' ? STRIPE_DESIGNER_PAYMENT_LINK : STRIPE_PRO_PAYMENT_LINK;
  const url = new URL(base);
  if (ctx.clerkUserId) url.searchParams.set('client_reference_id', ctx.clerkUserId);
  if (ctx.email) url.searchParams.set('prefilled_email', ctx.email);
  return url.toString();
}
