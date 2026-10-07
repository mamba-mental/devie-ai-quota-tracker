"use client";

import { type FormEvent, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import type {
  AccountApiProvider,
  DashboardState,
  Provider,
  ProviderConnection,
} from "@/lib/contracts";

import {
  connectAccountApi,
  getAccountApiCatalog,
  openExternalUrl,
} from "@/lib/desktop";

import { providerName } from "@/lib/labels";

import Button from "@/ui/Button";

import Dialog from "@/ui/Dialog";

import Field from "@/ui/Field";

import Input from "@/ui/Input";

import styles from "./LoginDialog.module.scss";

export default function AccountApiDialog({
  provider,
  open,
  renew,
  onOpenChange,
  onConnected,
}: {
  provider: Provider;
  open: boolean;
  renew?: ProviderConnection;
  onOpenChange: (open: boolean) => void;
  onConnected: (state: DashboardState) => void;
}) {
  const { t } = useTranslation();
  const [service, setService] = useState<AccountApiProvider>();

  const [apiKey, setApiKey] = useState("");

  const [secondarySecret, setSecondarySecret] = useState("");

  const [fields, setFields] = useState<Record<string, string>>({});

  const [error, setError] = useState<string>();

  const [busy, setBusy] = useState(false);

  const pending = useRef(false);

  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;

    void getAccountApiCatalog()
      .then((catalog) => {
        if (!mounted.current) return;

        const found = catalog.find((item) => item.id === provider);

        if (!found) setError(t("AccountApi.Unavailable"));
        else setService(found);
      })
      .catch(() => {
        if (mounted.current) setError(t("AccountApi.RequirementsFailed"));
      });

    return () => {
      mounted.current = false;
    };
  }, [provider, t]);

  const valid =
    service &&
    (!service.apiKeyRequired || apiKey.trim()) &&
    (!service.secondarySecretRequired || secondarySecret.trim()) &&
    service.fields.every(
      (field) => !field.required || fields[field.id]?.trim(),
    );

  function close(next: boolean) {
    if (pending.current) return;

    if (!next) {
      setApiKey("");
      setSecondarySecret("");
      setFields({});
    }

    onOpenChange(next);
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    if (!service || !valid || pending.current) return;

    pending.current = true;
    setBusy(true);
    setError(undefined);

    const submittedKey = apiKey.trim();
    const submittedSecret = secondarySecret.trim();

    setApiKey("");
    setSecondarySecret("");

    try {
      const next = await connectAccountApi(
        service.id,
        submittedKey,
        fields,
        submittedSecret,
        renew?.id,
      );

      if (mounted.current) {
        onConnected(next);
        onOpenChange(false);
      }
    } catch {
      if (mounted.current) setError(t("AccountApi.ConnectFailed"));
    } finally {
      pending.current = false;

      if (mounted.current) setBusy(false);
    }
  }

  const additionalCredentialLabel = t("AccountApi.AdditionalCredential");

  return (
    <Dialog.Root open={open} onOpenChange={close} size="sm">
      <Dialog.Portal>
        <Dialog.Backdrop />
        <Dialog.Popup>
          <Dialog.Header>
            <Dialog.Title>
              {renew
                ? t("AccountApi.ReconnectTitle", {
                    name: service?.name ?? providerName(provider),
                  })
                : t("AccountApi.ConnectTitle", {
                    name: service?.name ?? providerName(provider),
                  })}
            </Dialog.Title>
          </Dialog.Header>
          <Dialog.Body>
            <form className={styles.body} onSubmit={submit}>
              {!service && !error && (
                <p role="status">{t("AccountApi.LoadingRequirements")}</p>
              )}
              {service && (
                <>
                  <p className={styles.hint}>{service.description}</p>
                  <Button
                    variant="naked"
                    size="sm"
                    type="button"
                    onClick={() =>
                      void openExternalUrl(service.docsUrl).catch(() =>
                        setError(t("AccountApi.DocsFailed")),
                      )
                    }
                  >
                    {t("AccountApi.Docs")}
                  </Button>
                  {!service.apiKeyRequired && (
                    <p className={styles.hint}>{t("AccountApi.CliHint")}</p>
                  )}
                  {service.apiKeyRequired && (
                    <Field.Root>
                      <Field.Label>{t("AccountApi.KeyLabel")}</Field.Label>
                      <Input
                        required
                        type="password"
                        autoComplete="off"
                        autoCapitalize="none"
                        spellCheck={false}
                        disabled={busy}
                        value={apiKey}
                        onChange={(event) => setApiKey(event.target.value)}
                      />
                    </Field.Root>
                  )}
                  {service.fields.map((field) => (
                    <Field.Root key={field.id}>
                      <Field.Label>
                        {field.required
                          ? t("AccountApi.FieldRequired", {
                              label: field.label,
                            })
                          : t("AccountApi.FieldOptional", {
                              label: field.label,
                            })}
                      </Field.Label>
                      <Input
                        required={field.required}
                        type={field.secret ? "password" : "text"}
                        autoComplete="off"
                        autoCapitalize="none"
                        spellCheck={false}
                        disabled={busy}
                        value={fields[field.id] ?? ""}
                        onChange={(event) =>
                          setFields((current) => ({
                            ...current,
                            [field.id]: event.target.value,
                          }))
                        }
                      />
                    </Field.Root>
                  ))}
                  {(service.secondarySecretRequired ||
                    service.secondarySecretLabel) && (
                    <Field.Root>
                      <Field.Label>
                        {service.secondarySecretRequired
                          ? t("AccountApi.FieldRequired", {
                              label:
                                service.secondarySecretLabel ??
                                additionalCredentialLabel,
                            })
                          : t("AccountApi.FieldOptional", {
                              label:
                                service.secondarySecretLabel ??
                                additionalCredentialLabel,
                            })}
                      </Field.Label>
                      <Input
                        required={service.secondarySecretRequired}
                        type="password"
                        autoComplete="off"
                        autoCapitalize="none"
                        spellCheck={false}
                        disabled={busy}
                        value={secondarySecret}
                        onChange={(event) =>
                          setSecondarySecret(event.target.value)
                        }
                      />
                    </Field.Root>
                  )}
                  <Button disabled={busy || !valid} type="submit" size="sm">
                    {busy
                      ? t("AccountApi.Connecting")
                      : renew
                        ? t("AccountApi.Reconnect")
                        : t("AccountApi.Connect")}
                  </Button>
                </>
              )}
              {error && (
                <p className={styles.error} role="alert">
                  {error}
                </p>
              )}
            </form>
          </Dialog.Body>
          <Dialog.Footer>
            <Button
              disabled={busy}
              onClick={() => close(false)}
              variant="naked"
              size="sm"
            >
              {t("Common.Cancel")}
            </Button>
          </Dialog.Footer>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
