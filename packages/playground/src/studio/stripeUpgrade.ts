// Stripe Payment Link for LabWired Pro. The Payment Link itself is created
// in the Stripe dashboard; this module appends Stripe's documented query-
// string overrides so the webhook can map the resulting workspace back to a
// Clerk user.
//
// Stripe supports `client_reference_id` and `prefilled_email` on Payment Link
// URLs — both flow through to the resulting `checkout.session.completed`
// event without any API call.
//   https://stripe.com/docs/payment-links/url-parameters

export const STRIPE_PRO_PAYMENT_LINK = 'https://buy.stripe.com/bJeaEW56u3H16Tc3Gz5AQ03';

export interface StripeUpgradeContext {
  clerkUserId?: string | null;
  email?: string | null;
}

export function buildStripeUpgradeUrl(ctx: StripeUpgradeContext = {}): string {
  const url = new URL(STRIPE_PRO_PAYMENT_LINK);
  if (ctx.clerkUserId) url.searchParams.set('client_reference_id', ctx.clerkUserId);
  if (ctx.email) url.searchParams.set('prefilled_email', ctx.email);
  return url.toString();
}
