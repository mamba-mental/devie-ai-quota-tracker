"use client";

import { Maximize2, Pin, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import type {
  AccountDetail,
  DashboardState,
  ProviderConnection,
} from "@/lib/contracts";
import { formatDateTime } from "@/lib/date";
import { accountLabel, fullName, visibleWindows } from "@/lib/labels";
import Button from "@/ui/Button";
import ScrollArea from "@/ui/ScrollArea";
import AccountDetails from "./AccountDetails";
import styles from "./CompactView.module.scss";
import ProviderIcon from "./ProviderIcon";
import { quotaLevel, quotaStatusText, untilText } from "./QuotaBars";

function hasMeasuredQuota(connection: ProviderConnection): boolean {
  return visibleWindows(connection).some(
    (window) =>
      !window.paid &&
      !window.unlimited &&
      Number.isFinite(window.usedPercent) &&
      !(window.amount && window.amount.used === undefined),
  );
}

function detailPriority(detail: AccountDetail): number {
  if (
    detail.kind === "balance" ||
    /balance|remaining|available/.test(detail.key)
  )
    return 0;
  if (detail.kind === "plan" || /plan|tier/.test(detail.key)) return 1;
  if (detail.kind === "status") return 2;
  return 3;
}

function compactDetails(connection: ProviderConnection): AccountDetail[] {
  const hasQuota = hasMeasuredQuota(connection);
  return (connection.identity?.details ?? [])
    .filter(
      (detail) =>
        !hasQuota ||
        !(
          ["usage", "limit", "percent"].includes(detail.kind) ||
          /(?:^|[-_])(used|usage|limit|total|reset)(?:$|[-_])/.test(detail.key)
        ),
    )
    .toSorted((a, b) => detailPriority(a) - detailPriority(b))
    .slice(0, 2);
}

function CompactAccount({ connection }: { connection: ProviderConnection }) {
  const { t, i18n } = useTranslation();
  const windows = visibleWindows(connection);
  const status =
    connection.status === "stale"
      ? t("Connection.Status.Stale")
      : connection.status === "needs_login"
        ? t("Connection.Status.SignInNeeded")
        : connection.status === "error"
          ? t("Connection.Status.Error")
          : undefined;

  return (
    <article className={styles.account} aria-label={fullName(connection)}>
      <header className={styles.identity}>
        <ProviderIcon provider={connection.provider} size={18} />
        <strong className={styles.name} title={fullName(connection)}>
          {accountLabel(connection)}
        </strong>
        {status && (
          <span
            className={styles.status}
            title={connection.lastError}
            data-error={connection.status === "error" || undefined}
          >
            {status}
          </span>
        )}
      </header>
      {windows.map((window) => {
        const finite = Number.isFinite(window.usedPercent);
        const left = finite
          ? Math.max(0, Math.min(100, Math.round(100 - window.usedPercent)))
          : 0;
        const quotaStatus = quotaStatusText(t, window);
        const balance =
          window.amount && window.amount.used === undefined && !window.unlimited
            ? t("Quota.Balance", {
                balance:
                  `${new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 2 }).format(window.amount.total)} ${window.amount.unit ?? ""}`.trim(),
              })
            : undefined;
        const value = quotaStatus ?? balance ?? (finite ? `${left}%` : "—");
        const reset =
          window.resetsAt && Number.isFinite(Date.parse(window.resetsAt))
            ? window.resetsAt
            : undefined;
        return (
          <div
            className={styles.quota}
            key={window.key}
            data-level={quotaLevel(left)}
          >
            <span className={styles.label} title={window.label}>
              {window.label}
            </span>
            <strong className={styles.percent} title={value}>
              {value}
            </strong>
            <span
              className={styles.reset}
              title={
                reset
                  ? t("Quota.Reset.On", {
                      date: formatDateTime(reset, i18n.language),
                    })
                  : undefined
              }
            >
              {untilText(t, reset) ?? "—"}
            </span>
          </div>
        );
      })}
      <AccountDetails compact details={compactDetails(connection)} />
      {windows.length === 0 && !connection.identity?.details?.length && (
        <p className={styles.empty}>
          {connection.windows.length > 0
            ? t("Connection.AllBarsHidden")
            : (connection.lastError ?? t("Connection.NoQuotaData"))}
        </p>
      )}
    </article>
  );
}

export default function CompactView({
  state,
  refreshing,
  busy,
  pinned,
  onExpand,
  onRefresh,
  onPin,
}: {
  state: DashboardState;
  refreshing: boolean;
  busy: boolean;
  pinned: boolean;
  onExpand: () => void;
  onRefresh: () => void;
  onPin: () => void;
}) {
  const { t } = useTranslation();
  const connections = state.connections
    .filter((connection) => connection.enabled)
    .toSorted(
      (a, b) => Number(hasMeasuredQuota(b)) - Number(hasMeasuredQuota(a)),
    );

  return (
    <main className={styles.compact} aria-label={t("Compact.Title")}>
      <header className={styles.toolbar}>
        <Button
          disabled={busy}
          onClick={onExpand}
          size="sm"
          variant="secondary"
        >
          <Maximize2 aria-hidden size={14} />
          {t("Compact.Expand")}
        </Button>
        <Button
          disabled={refreshing}
          onClick={onRefresh}
          size="sm"
          variant="icon-naked"
          aria-label={t("Quota.RefreshQuotas")}
          title={t("Quota.RefreshQuotas")}
        >
          <RefreshCw
            aria-hidden
            size={15}
            className={refreshing ? styles.spinning : undefined}
          />
        </Button>
        <Button
          disabled={busy}
          onClick={onPin}
          size="sm"
          variant="secondary"
          aria-pressed={pinned}
          title={t("Compact.KeepOnTop")}
        >
          <Pin aria-hidden size={14} fill={pinned ? "currentColor" : "none"} />
          {t("Compact.KeepOnTop")}
        </Button>
      </header>
      <div className={styles.columns} aria-hidden>
        <span>{t("Compact.Title")}</span>
        <span>{t("Compact.Remaining")}</span>
        <span>{t("Compact.Reset")}</span>
      </div>
      <ScrollArea.Root className={styles.list}>
        <ScrollArea.Viewport className={styles.viewport}>
          {connections.map((connection) => (
            <CompactAccount key={connection.id} connection={connection} />
          ))}
          {connections.length === 0 && (
            <p className={styles.empty}>{t("Compact.Empty")}</p>
          )}
        </ScrollArea.Viewport>
        <ScrollArea.Scrollbar>
          <ScrollArea.Thumb />
        </ScrollArea.Scrollbar>
      </ScrollArea.Root>
    </main>
  );
}
