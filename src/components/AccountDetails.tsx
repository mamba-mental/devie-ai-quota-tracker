import { useTranslation } from "react-i18next";
import type { AccountDetail } from "@/lib/contracts";
import { formatDateTime } from "@/lib/date";
import styles from "./AccountDetails.module.scss";

const NUMERIC_KINDS = new Set([
  "money",
  "balance",
  "usage",
  "limit",
  "credits",
  "count",
  "number",
  "percent",
]);
const CURRENCY_UNITS = new Set([
  "USD",
  "CNY",
  "EUR",
  "GBP",
  "JPY",
  "CAD",
  "AUD",
  "CHF",
  "HKD",
  "SGD",
  "INR",
  "KRW",
  "BRL",
]);

function displayValue(detail: AccountDetail, locale: string): string {
  const raw = detail.value.trim();
  // Only explicit ISO timestamps are dates; numeric IDs and epoch-like values stay intact.
  if (
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(
      raw,
    ) &&
    Number.isFinite(Date.parse(raw))
  ) {
    return formatDateTime(raw, locale);
  }
  if (
    NUMERIC_KINDS.has(detail.kind) &&
    /^[+-]?(?:\d+\.?\d*|\.\d+)(?:e[+-]?\d+)?$/i.test(raw)
  ) {
    const value = Number(raw);
    if (Number.isFinite(value)) {
      return new Intl.NumberFormat(locale, {
        maximumFractionDigits:
          detail.kind === "money" ||
          CURRENCY_UNITS.has(detail.unit?.toUpperCase() ?? "")
            ? 2
            : 6,
      }).format(value);
    }
  }
  return detail.value;
}

/** Provider-reported values: balances stay balances, never synthetic percentages. */
export default function AccountDetails({
  details,
  compact = false,
}: {
  details?: AccountDetail[];
  compact?: boolean;
}) {
  const { i18n } = useTranslation();
  if (!details?.length) return null;
  return (
    <dl className={styles.details} data-compact={compact || undefined}>
      {details.map((detail) => (
        <div className={styles.row} key={detail.key}>
          <dt>{detail.label}</dt>
          <dd title={`${detail.value}${detail.unit ? ` ${detail.unit}` : ""}`}>
            {displayValue(detail, i18n.language)}
            {detail.unit ? ` ${detail.unit}` : ""}
          </dd>
        </div>
      ))}
    </dl>
  );
}
