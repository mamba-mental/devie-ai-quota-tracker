import type { AccountDetail } from "@/lib/contracts";
import styles from "./AccountDetails.module.scss";

/** Provider-reported values: balances stay balances, never synthetic percentages. */
export default function AccountDetails({
  details,
  compact = false,
}: {
  details?: AccountDetail[];
  compact?: boolean;
}) {
  if (!details?.length) return null;
  return (
    <dl className={styles.details} data-compact={compact || undefined}>
      {details.map((detail) => (
        <div className={styles.row} key={detail.key}>
          <dt>{detail.label}</dt>
          <dd>
            {detail.value}
            {detail.unit ? ` ${detail.unit}` : ""}
          </dd>
        </div>
      ))}
    </dl>
  );
}
