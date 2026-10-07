"use client";

import { useEffect, useState } from "react";

import type {
  AccountApiProvider,
  DashboardState,
  Provider,
} from "@/lib/contracts";

import { getAccountApiCatalog, isDesktop } from "@/lib/desktop";

import Button from "@/ui/Button";

import Input from "@/ui/Input";
import styles from "./AccountApiCatalog.module.scss";

import ProviderIcon from "./ProviderIcon";
import SettingRow from "./SettingRow";

export default function AccountApiCatalog({
  onOpen,
  state,
}: {
  onOpen: (provider: Provider) => void;
  state: DashboardState;
}) {
  const [catalog, setCatalog] = useState<AccountApiProvider[]>([]);

  const [query, setQuery] = useState("");

  const [error, setError] = useState(false);

  const [loading, setLoading] = useState(true);

  const [attempt, setAttempt] = useState(0);

  // biome-ignore lint/correctness/useExhaustiveDependencies: Retry intentionally reruns this catalog request.
  useEffect(() => {
    let active = true;

    setLoading(true);
    setError(false);

    void getAccountApiCatalog()
      .then((result) => {
        if (active) setCatalog(result);
      })
      .catch(() => {
        if (active) setError(true);
      })
      .finally(() => {
        if (active) setLoading(false);
      });

    return () => {
      active = false;
    };
  }, [attempt]);

  const matches = catalog.filter((provider) =>
    `${provider.name} ${provider.category} ${provider.description}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );

  const categories = [...new Set(matches.map((provider) => provider.category))];

  return (
    <SettingRow.Group plain title="API accounts and balances">
      <div className={styles.catalog}>
        <p className={styles.hint}>
          Connect account services to see the balances, usage and plan
          information they report. Available details vary by provider.
        </p>
        {!isDesktop() && (
          <p className={styles.hint}>
            Open the desktop app to browse and connect API accounts.
          </p>
        )}
        {loading && <p role="status">Loading account services…</p>}
        {error && (
          <div role="alert">
            <p>Account services could not be loaded.</p>
            <Button size="sm" onClick={() => setAttempt((value) => value + 1)}>
              Retry
            </Button>
          </div>
        )}
        {catalog.length > 0 && (
          <Input
            aria-label="Search API services"
            placeholder="Search providers or categories"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        )}
        {categories.map((category) => (
          <section key={category} className={styles.category}>
            <h3>{category.replaceAll("_", " ")}</h3>
            <div className={styles.grid}>
              {matches
                .filter((provider) => provider.category === category)
                .map((provider) => {
                  const accounts = state.connections.filter(
                    (connection) => connection.provider === provider.id,
                  ).length;

                  return (
                    <button
                      type="button"
                      className={styles.provider}
                      key={provider.id}
                      onClick={() => onOpen(provider.id)}
                    >
                      <span className={styles.name}>
                        <ProviderIcon provider={provider.id} />
                        <strong>{provider.name}</strong>
                      </span>
                      <span>{provider.description}</span>
                      <small>
                        {accounts
                          ? `${accounts} saved account${accounts === 1 ? "" : "s"} · `
                          : ""}
                        {provider.apiKeyRequired
                          ? "API key"
                          : "Local command-line sign-in"}
                      </small>
                    </button>
                  );
                })}
            </div>
          </section>
        ))}
        {catalog.length > 0 && matches.length === 0 && (
          <p>No services match this search.</p>
        )}
      </div>
    </SettingRow.Group>
  );
}
