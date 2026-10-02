// Shared email frame. Templates are plain HTML strings (no React Email dependency); the .tsx
// extension keeps DESIGN.md 5.11's layout. Everything user-supplied goes through `esc`.

import { brand } from "../lib/brand";

export function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

export interface Frame {
  title: string;
  /** Inner HTML (already escaped). */
  body: string;
  /** Plain-text alternative. */
  text: string;
}

export function layout(frame: Frame): { html: string; text: string } {
  const html = `<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>${esc(frame.title)}</title></head>
<body style="margin:0;background:#F6F4F0;font-family:Inter,-apple-system,Segoe UI,sans-serif;color:#1C1924;">
  <table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#F6F4F0;padding:24px 12px;">
    <tr><td align="center">
      <table role="presentation" width="560" cellpadding="0" cellspacing="0" style="max-width:560px;width:100%;background:#FFFFFF;border:1px solid #ECE8E2;border-radius:14px;">
        <tr><td style="padding:28px 32px 8px 32px;font-size:22px;font-weight:700;color:#1C1924;">${esc(brand.wordmark)}</td></tr>
        <tr><td style="padding:8px 32px 28px 32px;font-size:15px;line-height:23px;">${frame.body}</td></tr>
      </table>
      <p style="font-size:12px;line-height:18px;color:#8A8496;max-width:560px;margin:16px 0 0 0;">
        ${esc(brand.name)} is made by ${esc(brand.seller)}. Questions: <a href="mailto:${esc(brand.urls.support_email)}" style="color:#5C5768;">${esc(brand.urls.support_email)}</a>
      </p>
    </td></tr>
  </table>
</body>
</html>`;
  const text = `${frame.text}\n\n--\n${brand.name} is made by ${brand.seller}. Questions: ${brand.urls.support_email}\n`;
  return { html, text };
}

export function button(href: string, label: string): string {
  return `<p style="margin:20px 0;"><a href="${esc(href)}" style="display:inline-block;background:${brand.accent};color:#FFFFFF;text-decoration:none;font-weight:600;padding:11px 20px;border-radius:10px;">${esc(label)}</a></p>`;
}

export function keyBlock(key: string): string {
  return `<p style="margin:16px 0;font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:20px;letter-spacing:1px;background:#EEEBE6;padding:14px 16px;border-radius:10px;text-align:center;">${esc(key)}</p>`;
}

export function para(s: string): string {
  return `<p style="margin:0 0 14px 0;">${s}</p>`;
}
