import type { Metadata } from "next";

import { Page } from "@/components/site-chrome";

import { AccountClient } from "./account-client";

export const metadata: Metadata = { title: "Account", robots: { index: false } };

export default function AccountPage() {
  return (
    <Page>
      <AccountClient />
    </Page>
  );
}
