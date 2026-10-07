# Account services

Devie connects to **46 account services** to display the usage, balances, limits, and plan information their reporting APIs expose. Choose a service in **Settings → API accounts and balances**, enter its reporting credentials and required account identifiers, then select **Connect account**.

A service being listed means a reader is implemented. It does not guarantee that every account, plan, key, or region has access to the provider's reporting API. The provider links below explain setup and permissions.

## Connect and read your account

1. Open the desktop app's Settings and search the account-service catalog by provider or category.
2. Select the service. Enter the requested key and account fields, or select an existing CLI profile for the two CLI-based services. Required and optional fields are labeled in the dialog.
3. Select **Connect account**. Devie reads the account report **before saving** the credentials and connection. A failed report leaves the connection unsaved; correct the fields or permissions and retry. Reconnect follows the same validation path.
4. Read the resulting account card. **Compact view** presents enabled accounts in the same desktop window; **Expand** returns to the full view. **Keep on top** controls the compact window's visibility above other applications.

Windows stores saved credentials using **current-user DPAPI encryption** in the application data directory. Unix uses private credential files with restricted permissions. CLI connections use credentials already managed by the provider's local CLI; Devie does not install the CLI or initiate its sign-in. API keys are entered manually in the connection dialog, and service settings cannot supply arbitrary remote API hosts.

## What the values mean

| Reported information | Meaning |
|---|---|
| **Quota / allowance** | Usage against a provider-reported cap. A percentage needs a valid denominator. A returned reset time applies to that specific window. |
| **Balance / credits** | Remaining money, points, credits, or another named unit. A balance alone does not establish a monthly percentage. |
| **Cost / spend** | Billed or metered usage for the stated period. It does not establish remaining funds. Provider reporting can lag actual activity. |
| **Plan / status / dates** | Explicit subscription metadata, access status, expiry, or billing dates. An API rate-limit reset is not automatically a subscription renewal. |

Missing values remain unavailable rather than becoming zero. Different currencies and units remain separate. A provider can return useful primary data while an optional supplementary report is unavailable; the card identifies that limitation where supported.

An API connection does **not** generally reveal the billing or quotas of the provider's personal chat or coding subscription. For example, OpenAI API costs are separate from ChatGPT, Anthropic API costs are separate from Claude subscriptions, and Moonshot API balance is separate from Kimi Code. Dedicated subscription readers below expose only their documented plan reports.

## Subscription and coding services — 6

| Service and exact ID | Information displayed by the reader | Credential and account requirements |
|---|---|---|
| [Chutes](https://github.com/chutesai/chutes-api/blob/main/api/user/router.py) — `api:chutes` | Subscription active status; four-hour and monthly allowance usage, caps, and resets; subscription price and cycle dates. Explicit uncapped custom monthly allowance is recognized. | Chutes API key. No extra ID. This reader uses subscription usage, not a generic prepaid-wallet query. |
| [MiniMax](https://github.com/MiniMax-AI/cli) — `api:minimax` | Token Plan window and weekly allowances, or API wallet balances for PAYG keys. Wallet currency is left unspecified because the official contract does not report it. | Global MiniMax key. Keys beginning `sk-api-` use the official account-balance route; other keys use the Token Plan route. Ambiguous legacy counts are not converted into quota. |
| [Z.ai Coding Plan](https://docs.z.ai/devpack/extension/usage-query-plugin) — `api:zai` | Five-hour token allowance and monthly MCP allowance percentages; MCP counts when returned. Credit-limit percentages without a documented window remain details. | Coding Plan API key; optional `region`: `global` (default) or `cn`. |
| [Poe](https://creator.poe.com/docs/resources/usage-api) — `api:poe` | Available plan and add-on points combined. | Poe API key. The aggregate balance does not identify a plan-only quota or renewal date. |
| [Moonshot / Kimi API](https://platform.kimi.ai/docs/api/balance) — `api:moonshot` | Available, cash, and voucher API balance in the region's currency. | Moonshot API key; optional `region`: `global` (default, USD) or `cn` (CNY). |
| [Kimi Code (local)](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/reference/server-api.md) — `api:kimi-code` | Available five-hour, weekly, monthly-total, and monthly-coding quota windows; optional plan name; extra-usage wallet and monthly charge limit. | Existing local Kimi Code server, signed in to a managed account, and its bearer token. Optional `port`, default `58627`; connection stays on `127.0.0.1`. |

## Voice and media services — 6

| Service and exact ID | Information displayed by the reader | Credential and account requirements |
|---|---|---|
| [ElevenLabs](https://elevenlabs.io/docs/api-reference/user/subscription/get) — `api:elevenlabs` | Tier, status, billing/refresh periods, characters used and allowance, reset, and current overage when returned. Valid character caps produce quota windows. | ElevenLabs API key with user/subscription read access. |
| [Fish Audio](https://docs.fish.audio/api-reference/openapi.json) — `api:fish-audio` | Package type/status, package allowance/balance, extra package balance, period end, expiry/cancellation; separate API wallet balance and cumulative top-ups. | Fish API key; optional `team_id` for the API wallet. Package units and API credits are kept separate; no invented package reset percentage. |
| [fal.ai](https://fal.ai/docs/platform-apis/v1/account/billing) — `api:fal` | API credit balance and reported currency. | **ADMIN** API key. |
| [Runware](https://runware.ai/docs/models-api/account-management) — `api:runware` | Account/free balance, reported spend and request counts across available usage periods. | Runware API key authorized for account management. Uses the read-only `getDetails` operation, not image generation. |
| [ImageRouter](https://docs.imagerouter.io/api-reference/credits/) — `api:imagerouter` | USD credit balance, total credit spend, and total deposits. | ImageRouter API key. |
| [Deepgram](https://developers.deepgram.com/reference/manage/billing/list) — `api:deepgram` | Project balances in each returned unit/currency. | Project key with **`billing:read`**; required `project_id`. |

## API balances and billing reports — 11

### Routers and inference wallets

| Service and exact ID | Information displayed by the reader | Credential and account requirements |
|---|---|---|
| [OpenRouter](https://openrouter.ai/docs/api_reference/limits) — `api:openrouter` | Key budget/remaining budget; lifetime and UTC day/week/month spend; free-model daily request counts; account purchased/spent/derived remaining credits when accessible. | API key; optional secondary **management API key** for account credits. Key caps and account balance are distinct. |
| [DeepSeek](https://api-docs.deepseek.com/api/get-user-balance/) — `api:deepseek` | Total, unexpired granted, and topped-up balance, keeping USD and CNY separate. | DeepSeek API key. |
| [Vercel AI Gateway](https://github.com/vercel/ai/blob/main/packages/gateway/src/gateway-fetch-metadata.ts) — `api:vercel-ai-gateway` | Team Gateway balance and lifetime Gateway spend. | AI Gateway API key preferred; optional `team_id`. This is not Vercel hosting or v0 billing. |
| [Requesty](https://docs.requesty.ai/api-reference/endpoint/manage-org-get) — `api:requesty` | Organization USD balance and reported month spend when the supplementary usage request succeeds. | Key with organization-management reporting access; optional `month` in `YYYY-MM`, otherwise current UTC month. |
| [SiliconFlow](https://docs.siliconflow.com/en/api-reference/userinfo/get-user-info) — `api:siliconflow` | Granted, top-up, and total balance. | Region-matching API key; required `region`: `cn` (CNY) or `global` (USD). |
| [Novita](https://novita.ai/docs/api-reference/basic-get-user-balance) — `api:novita` | Available and top-up balance, permitted debt limit, pending charges, and outstanding invoices. | Novita API key. Provider amounts are converted from ten-thousandths of USD; the debt limit is not prepaid credit. |

### Separate credit units and compute reports

| Service and exact ID | Information displayed by the reader | Credential and account requirements |
|---|---|---|
| [Venice](https://docs.venice.ai/api-reference/endpoint/billing/balance) — `api:venice` | USD, DIEM, and bundled balances; API access status, key expiry, next DIEM epoch. Admin billing adds account balances, earned credits, and DIEM epoch allocation. | Inference or admin API key; optional secondary **admin key** for the fuller billing report. DIEM epoch is not subscription renewal. |
| [Hugging Face ZeroGPU](https://huggingface.co/docs/huggingface_hub/main/guides/manage-spaces) — `api:huggingface-zerogpu` | Base and remaining GPU-seconds, paid over-quota usage, and returned reset time. | Token with **Billing → Read billing usage and payment method status**, or OAuth `read-billing`. This does not report general inference-credit balance. |
| [Together AI](https://docs.together.ai/reference/billing-usage) — `api:together` | Organization month spend after complete pagination. Current-month data can lag one hour; earlier months can lag 24 hours. | API key; required `organization_id`; optional `month` (`YYYY-MM`). Billing API **beta access must be enabled for the organization**. No prepaid balance is read. |
| [Fireworks AI](https://docs.fireworks.ai/api-reference/get-billing-summary) — `api:fireworks` | Reported month costs, separated by currency. | Account-authorized API key; required `account_id`; optional `month` (`YYYY-MM`). Cost summaries are not remaining credits. |
| [Runpod](https://docs.runpod.io/runpodctl/reference/runpodctl-user) — `api:runpod` | Credit balance, current spend per hour, and hourly spending limit. | Runpod API key. Devie uses a fixed read-only GraphQL query directly; no Runpod CLI is required. |

## Business and automation services — 9

| Service and exact ID | Information displayed by the reader | Credential and account requirements |
|---|---|---|
| [Apify](https://docs.apify.com/api/v2/users-me-limits-get) — `api:apify` | Plan, monthly included USD usage, cycle usage, spending limit, and cycle end; valid spending caps produce a usage window. | Apify account token with user and account-limit read access. |
| [Firecrawl](https://docs.firecrawl.dev/api-reference/endpoint/credit-usage) — `api:firecrawl` | Remaining credits, plan credits, and billing-period boundaries. | Firecrawl API key. Billing-period dates alone do not establish a quota reset. |
| [Hunter](https://hunter.io/api-documentation/) — `api:hunter` | Plan and reset date; credit/search/verification buckets with remaining, used, and allocation values when returned. | Hunter API key. Uses the provider's account endpoint and prescribed query-key authentication. |
| [Context7](https://context7.com/docs/api-reference/usage/get-api-usage) — `api:context7` | Plan, member requests/included requests, team extra-request cost and spending limit, reset. | Context7 API key. |
| [Cloudflare](https://developers.cloudflare.com/api/typescript/resources/accounts/subresources/subscriptions/methods/get/) — `api:cloudflare` | Account subscriptions: plan, status, billing frequency, next billing due, and price/currency. | Token with account subscription read permission; required `account_id` (32 hexadecimal characters). Not a Workers AI usage report. |
| [Make](https://developers.make.com/api-documentation/api-reference/organizations) — `api:make` | Service plan, next usage reset, and operations/centicredits/data transfer in the returned 30-day history. | Organization-authorized token; required `organization_id` and `zone`: `eu1`, `eu2`, `us1`, or `us2`. Rolling history is not treated as a billing-cycle quota. |
| [Todoist](https://developer.todoist.com/api/v1/) — `api:todoist` | Pro entitlement active status, subscription status, and Pro expiry excluding grace period. | Personal token or a token with `user:read`. Does not report subscription price or a task allowance. |
| [GitHub Billing](https://docs.github.com/en/rest/billing/usage) — `api:github-billing` | Current-month net metered usage for the personal account. | Token with billing access; required `username`. Excludes organization-billed licenses and does not replace the dedicated Copilot quota connection. |
| [AWS Free Tier](https://docs.aws.amazon.com/aws-cost-management/latest/APIReference/API_freetier_GetFreeTierUsage.html) — `api:aws-billing` | Returned Free Tier usage and limits by service. | Installed AWS CLI, an existing authenticated `profile` (required), and `freetier:GetFreeTierUsage`. No key is entered in Devie. This is not total AWS spend or Bedrock account balance. |

## Organization AI and cloud reporting — 8

| Service and exact ID | Information displayed by the reader | Credential and account requirements |
|---|---|---|
| [OpenAI API](https://platform.openai.com/docs/api-reference/usage) — `api:openai-api` | Organization reported API costs for the current UTC month, with report period and separate currencies. | **Admin API key** with organization cost-report access. No ChatGPT subscription information or prepaid balance. |
| [Anthropic API](https://platform.claude.com/docs/en/manage-claude/usage-cost-api) — `api:anthropic-api` | Current-month organization API costs for completed UTC days. Reporting can lag; provider Priority Tier exclusions are identified. | **Admin API key**. No Claude consumer subscription billing or remaining API credit balance. |
| [xAI API](https://docs.x.ai/developers/rest-api-reference/management/billing) — `api:xai-api` | Signed prepaid ledger balance, plus available postpaid cap and current-month API spend from supplementary reports. | **Management API key** and required `team_id`. The ledger retains the provider's sign convention. Not a personal Grok plan quota. |
| [Mistral API](https://docs.mistral.ai/api/endpoint/beta/admin/billing) — `api:mistral-api` | Monthly limit/payment status, configured spend and throughput limits, and reported usage when supplied. | **Admin API key**. Amounts with unspecified major/minor currency scale remain labeled provider units; no Le Chat renewal information. |
| [Perplexity Enterprise](https://docs.perplexity.ai/docs/admin/computer-analytics-api) — `api:perplexity-enterprise` | Enterprise Computer credits used in completed reporting hours this month, with period and reporting-lag caveat. | **Enterprise analytics key**. Usage, not remaining balance or personal Pro quota. |
| [Groq Enterprise](https://console.groq.com/docs/prometheus-metrics) — `api:groq-enterprise` | Input/output token rates averaged over five minutes, from Enterprise metrics. | **Enterprise key** with metrics access. No monetary balance, spend, or renewal is read. |
| [Azure Subscription Billing](https://learn.microsoft.com/en-us/rest/api/cost-management/query/usage?view=rest-cost-management-2025-03-01) — `api:azure-billing` | Month-to-date subscription costs across Azure services, separated by currency; credential-expiry guidance. | Current **Entra bearer token** with Cost Management read access and required `subscription_id`. Reconnect when the token expires. Not limited to Azure AI usage. |
| [Alibaba Cloud Billing](https://help.aliyun.com/en/user-center/developer-reference/api-queryaccountbalance) — `api:alibaba-billing` | Whole-account available balance and cash balance in the reported currency. | Installed/authenticated **`aliyun` CLI**, billing read permission; optional `profile`; optional `region`: `cn-hangzhou` (default) or `ap-southeast-1`. No key is entered in Devie. Not Model Studio free-token quota. |

## Additional API services — 6

| Service and exact ID | Information displayed by the reader | Credential and account requirements |
|---|---|---|
| [NavyAI](https://api.navy/docs/usage-statistics) — `api:navyai` | Plan, daily token allowance/used/remaining/reset, and per-minute request limits and remaining counts. A valid daily cap produces a quota window. | NavyAI API key. |
| [AINative](https://docs.ainative.studio/docs/billing/credits/) — `api:ainative` | Credit allowance/used/remaining, period, plan name, subscription status, billing cycle, and renewal when returned. | AINative API key authorized for credits and subscription reads. Credit-period end is not automatically a purchased-credit reset. |
| [Pollinations](https://gen.pollinations.ai/docs) — `api:pollinations` | Pollen available to the key; account total/earned/paid Pollen when permitted. | Pollinations API key; **`account:usage`** permission exposes account-level totals. A key budget is not the whole account balance. |
| [AI Horde](https://aihorde.net/api/) — `api:ai-horde` | Kudos balance, recurring monthly kudos, last monthly award, and concurrency limit. | AI Horde API key. Kudos are priority credits, not currency or a monthly spend cap. |
| [BazaarLink](https://bazaarlink.ai/docs/api/reference/account/get-credits) — `api:bazaarlink` | Available USD balance, lifetime spend, month spend/requests/tokens, and returned usage-range dates. | BazaarLink API key. Available credits are already remaining balance; lifetime spend is not subtracted again. |
| [Routeway](https://docs.routeway.ai/account-management/balance-and-usage) — `api:routeway` | USD credit balance. | **Management key**; an inference key is insufficient. |

## Troubleshooting and scope

- **Connection rejected:** check the key type, billing/read scopes, region, and required IDs. A key that can generate a model response may still lack reporting permission. An organization-only endpoint cannot be assumed to work for a personal account.
- **CLI unavailable:** install and authenticate AWS CLI or Alibaba Cloud CLI separately, then choose the existing profile. Devie runs the fixed read operation and does not open an interactive sign-in flow.
- **No data or stale data:** respect the displayed reporting period. Some billing reports contain only completed days/hours. Empty or incomplete reports do not prove that all usage is zero; bounded pagination failures do not save partial totals.
- **A field is absent:** the connected account may not return it. A working balance reader does not imply invoice access, cancellation controls, renewal dates, or consumer-plan visibility. Account-service readers perform reporting operations and do not buy credits, change subscriptions, or send inference requests.

This reference describes the implemented readers and catalog. A successful connection establishes access for that credential at that time; it is not a guarantee of future provider availability or billing accuracy beyond the provider's report.
