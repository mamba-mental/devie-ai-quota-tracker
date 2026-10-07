"use client";

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

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
  const { t } = useTranslation();
  const [catalog, setCatalog] = useState<AccountApiProvider[]>([]);

  const [query, setQuery] = useState("");

  const [error, setError] = useState(false);

  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!loading) return;
    let active = true;

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
  }, [loading]);

  const matches = catalog.filter((provider) =>
    `${provider.name} ${provider.category} ${provider.description}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );

  const categories = [...new Set(matches.map((provider) => provider.category))];

  return (
    <SettingRow.Group plain title={t("AccountApi.Title")}>
      <div className={styles.catalog}>
        <p className={styles.hint}>{t("AccountApi.Description")}</p>
        {!isDesktop() && (
          <p className={styles.hint}>{t("AccountApi.DesktopOnly")}</p>
        )}
        {loading && <p role="status">{t("AccountApi.LoadingCatalog")}</p>}
        {error && (
          <div role="alert">
            <p>{t("AccountApi.CatalogFailed")}</p>
            <Button size="sm" onClick={() => setLoading(true)}>
              {t("Common.TryAgain")}
            </Button>
          </div>
        )}
        {catalog.length > 0 && (
          <Input
            aria-label={t("AccountApi.Search")}
            placeholder={t("AccountApi.SearchPlaceholder")}
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
                          ? t("AccountApi.SavedAccounts", { count: accounts })
                          : ""}
                        {provider.apiKeyRequired
                          ? t("AccountApi.KeyMethod")
                          : t("AccountApi.CliMethod")}
                      </small>
                    </button>
                  );
                })}
            </div>
          </section>
        ))}
        {catalog.length > 0 && matches.length === 0 && (
          <p>{t("AccountApi.NoMatches")}</p>
        )}
      </div>
    </SettingRow.Group>
  );
}
