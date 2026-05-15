// LabWired API Worker — Stripe webhook signature verification

import Stripe from 'stripe';
import type { Env } from './types.js';

/**
 * Verify the Stripe-Signature header and return the parsed event.
 * Throws if the signature is invalid or the body cannot be parsed.
 */
export async function verifyStripeWebhook(
  request: Request,
  env: Env,
): Promise<Stripe.Event> {
  const signature = request.headers.get('stripe-signature');
  if (!signature) {
    throw new Error('Missing Stripe-Signature header');
  }

  // Read the raw body once — we need the exact bytes Stripe signed.
  const rawBody = await request.text();

  const stripe = new Stripe(env.STRIPE_SECRET_KEY, {
    apiVersion: '2025-04-30.basil',
    httpClient: Stripe.createFetchHttpClient(),
  });

  // constructEventAsync is the Workers-compatible (async) variant.
  const event = await stripe.webhooks.constructEventAsync(
    rawBody,
    signature,
    env.STRIPE_WEBHOOK_SECRET,
  );

  return event;
}

/** Construct a Stripe client for non-webhook calls (e.g. subscription fetches). */
export function getStripeClient(env: Env): Stripe {
  return new Stripe(env.STRIPE_SECRET_KEY, {
    apiVersion: '2025-04-30.basil',
    httpClient: Stripe.createFetchHttpClient(),
  });
}
