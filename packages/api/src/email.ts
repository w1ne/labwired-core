// LabWired API Worker — Resend transactional email

import type { Env } from './types.js';

const RESEND_API_URL = 'https://api.resend.com/emails';

interface ResendSendRequest {
  from: string;
  to: string[];
  subject: string;
  html: string;
}

/**
 * Send the post-purchase onboarding email with the customer's API key.
 * Falls back to console.log if RESEND_API_KEY is unset (e.g. local dev).
 */
export async function sendOnboardingEmail(
  env: Env,
  to: string,
  apiKey: string,
  workspaceId: string,
): Promise<void> {
  const subject = 'Your LabWired Pro API key is ready';
  const html = buildOnboardingHtml(apiKey, workspaceId);

  if (!env.RESEND_API_KEY) {
    console.log(
      `[email stub] Would send onboarding email to ${to} with key ${apiKey} workspace ${workspaceId}`,
    );
    return;
  }

  const body: ResendSendRequest = {
    from: env.FROM_EMAIL || 'onboarding@labwired.com',
    to: [to],
    subject,
    html,
  };

  let resp: Response;
  try {
    resp = await fetch(RESEND_API_URL, {
      method: 'POST',
      headers: {
        Authorization: `Bearer ${env.RESEND_API_KEY}`,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(body),
    });
  } catch (err) {
    console.error('Resend fetch error:', err);
    return;
  }

  if (!resp.ok) {
    const errText = await resp.text().catch(() => '(unreadable)');
    console.error(`Resend returned ${resp.status}: ${errText}`);
  } else {
    console.log(`Onboarding email sent to ${to}`);
  }
}

function buildOnboardingHtml(apiKey: string, workspaceId: string): string {
  return `<!DOCTYPE html>
<html lang="en">
<head><meta charset="UTF-8"><title>LabWired Pro — API Key</title></head>
<body style="font-family: 'Helvetica Neue', Helvetica, Arial, sans-serif; background: #0d0e12; color: #e8e9ec; padding: 40px 20px; margin: 0;">
  <table width="100%" cellpadding="0" cellspacing="0" style="max-width: 600px; margin: 0 auto;">
    <tr><td>
      <p style="font-size: 24px; font-weight: 700; color: #7cffb2; margin-bottom: 8px;">LabWired Pro</p>
      <p style="font-size: 16px; color: #a0a3aa; margin-top: 0;">Your workspace is ready.</p>

      <hr style="border: 0; border-top: 1px solid #2a2c35; margin: 24px 0;" />

      <p style="font-size: 15px; margin-bottom: 8px;">Your API key:</p>
      <pre style="background: #1a1c23; border: 1px solid #2a2c35; border-radius: 6px; padding: 16px; font-size: 14px; color: #7cffb2; word-break: break-all; margin: 0 0 24px;">${apiKey}</pre>

      <p style="font-size: 14px; color: #a0a3aa; margin-bottom: 4px;">
        Workspace ID: <code style="color: #e8e9ec;">${workspaceId}</code>
      </p>

      <hr style="border: 0; border-top: 1px solid #2a2c35; margin: 24px 0;" />

      <p style="font-size: 15px; font-weight: 600; margin-bottom: 8px;">Quick start</p>
      <p style="font-size: 14px; color: #a0a3aa; margin-bottom: 4px;">Add to your CI environment:</p>
      <pre style="background: #1a1c23; border: 1px solid #2a2c35; border-radius: 6px; padding: 16px; font-size: 13px; color: #e8e9ec; margin: 0 0 16px;">LABWIRED_API_KEY=${apiKey}</pre>

      <p style="font-size: 14px; color: #a0a3aa; margin-bottom: 4px;">In your GitHub Actions workflow:</p>
      <pre style="background: #1a1c23; border: 1px solid #2a2c35; border-radius: 6px; padding: 16px; font-size: 13px; color: #e8e9ec; margin: 0 0 24px;">- name: Run LabWired simulation
  uses: w1ne/labwired/.github/actions/labwired-test@main
  with:
    script: tests/firmware-regression.yaml
    output_dir: test-results
  env:
    LABWIRED_API_KEY: \${{ secrets.LABWIRED_API_KEY }}</pre>

      <p style="font-size: 14px; color: #a0a3aa;">
        Keep this key secret — treat it like a password. If you need to rotate it, reply to this email.
      </p>

      <hr style="border: 0; border-top: 1px solid #2a2c35; margin: 24px 0;" />
      <p style="font-size: 12px; color: #555860;">
        LabWired · Deterministic firmware simulation ·
        <a href="https://github.com/w1ne/labwired" style="color: #7cffb2;">GitHub</a>
      </p>
    </td></tr>
  </table>
</body>
</html>`;
}
