import { useEffect, useState } from 'react'
import { defaultCapabilities } from '../lib/defaults'
import { formatBytes, type DeviceCapabilities } from '../lib/model-catalog'
import {
  checkProviderConnection,
  deleteProviderCredential,
  getDeviceCapabilities,
  runningInTauri,
  runningOnAndroid,
  saveProviderCredential,
} from '../lib/runtime'
import { inferTransportLocation } from '../lib/storage'
import type { AppData, ProviderProfile } from '../types'
import { useExternalModels } from './external-models'
import { Icon } from './icons'
import { nativeRuntimeLabel } from './ModelsScreen'
import { useDialogs } from './overlays'
import { EmptyState, LocationBadge, Section, uid } from './ui'

type Update = (change: (current: AppData) => AppData) => void

export function ConnectionsSettings({ data, update }: { data: AppData; update: Update }) {
  const dialogs = useDialogs()
  const [formOpen, setFormOpen] = useState(false)
  const [editing, setEditing] = useState<ProviderProfile | null>(null)
  const [name, setName] = useState('Local llama.cpp server')
  const [baseUrl, setBaseUrl] = useState('http://127.0.0.1:8080/v1')
  const [kind, setKind] = useState<ProviderProfile['kind']>('openai-compatible')
  const [apiKey, setApiKey] = useState('')
  const [checking, setChecking] = useState<string | null>(null)
  const location = inferTransportLocation(baseUrl.trim())

  function openForm(provider?: ProviderProfile) {
    setEditing(provider ?? null)
    setName(provider?.name ?? 'Local llama.cpp server')
    setBaseUrl(provider?.baseUrl ?? 'http://127.0.0.1:8080/v1')
    setKind(provider?.kind ?? 'openai-compatible')
    setApiKey('')
    setFormOpen(true)
  }

  async function save() {
    const trimmedName = name.trim()
    const url = baseUrl.trim()
    if (!trimmedName || !url) return
    const providerId = editing?.id ?? uid('provider')
    const apiKeyRef = apiKey.trim() ? (editing?.apiKeyRef ?? uid('credential')) : editing?.apiKeyRef
    if (apiKeyRef && apiKey.trim()) {
      try {
        await saveProviderCredential(apiKeyRef, apiKey.trim())
      } catch (error) {
        void dialogs.notify(
          error instanceof Error ? error.message : 'Could not save the API key securely.',
          'API key not saved',
        )
        return
      }
    }
    const transport = inferTransportLocation(url)
    const provider: ProviderProfile = {
      ...(editing ?? {}),
      id: providerId,
      name: trimmedName,
      kind,
      baseUrl: url,
      locality: transport === 'remote' ? 'remote' : transport === 'unknown' ? 'unknown' : 'local',
      transportLocation: transport,
      apiKeyRef,
      enabled: editing?.enabled ?? true,
      status: 'unknown',
      capabilities: {
        ...(editing?.capabilities ??
          data.providers[0]?.capabilities ?? {
            ...defaultCapabilities,
            generationParameters: ['temperature'],
          }),
      },
      deviceId: undefined,
      deviceLinkFingerprint: undefined,
    }
    update((current) => ({
      ...current,
      providers: current.providers.some((item) => item.id === provider.id)
        ? current.providers.map((item) => (item.id === provider.id ? provider : item))
        : [...current.providers, provider],
    }))
    setApiKey('')
    setFormOpen(false)
    setEditing(null)
  }

  async function test(provider: ProviderProfile) {
    setChecking(provider.id)
    try {
      await checkProviderConnection(provider)
      update((current) => ({
        ...current,
        providers: current.providers.map((item) =>
          item.id === provider.id ? { ...item, status: 'connected' } : item,
        ),
      }))
    } catch (error) {
      update((current) => ({
        ...current,
        providers: current.providers.map((item) =>
          item.id === provider.id ? { ...item, status: 'offline' } : item,
        ),
      }))
      void dialogs.notify(
        error instanceof Error ? error.message : 'Provider connection failed.',
        `Couldn’t reach ${provider.name}`,
      )
    } finally {
      setChecking(null)
    }
  }

  async function remove(provider: ProviderProfile) {
    const dependent = data.models.filter((model) => model.providerId === provider.id).length
    const confirmed = await dialogs.confirm({
      title: `Remove ${provider.name}?`,
      message: `${dependent} model profile(s) from this connection will become unavailable.${provider.apiKeyRef ? ' Its saved API key is deleted from the secure vault.' : ''}`,
      confirmLabel: 'Remove connection',
      danger: true,
    })
    if (!confirmed) return
    if (provider.apiKeyRef && runningInTauri) {
      try {
        await deleteProviderCredential(provider.apiKeyRef)
      } catch (error) {
        void dialogs.notify(
          error instanceof Error ? error.message : 'Could not remove the provider credential.',
        )
        return
      }
    }
    update((current) => ({
      ...current,
      providers: current.providers.filter((item) => item.id !== provider.id),
      models: current.models.map((model) =>
        model.providerId === provider.id ? { ...model, status: 'not-found' } : model,
      ),
    }))
  }

  const providers = data.providers.filter((provider) => provider.kind !== 'juniper-local')

  return (
    <>
      <Section
        title="Model providers"
        description="Connect Ollama, a llama.cpp server, or an OpenAI-compatible endpoint. Juniper labels where each one runs."
        action={
          !formOpen && (
            <button className="button secondary" onClick={() => openForm()}>
              <Icon name="plus" size={18} />
              Add connection
            </button>
          )
        }
      >
        {formOpen && (
          <form
            className="provider-form"
            onSubmit={(event) => {
              event.preventDefault()
              void save()
            }}
          >
            <h3>{editing ? `Edit ${editing.name}` : 'New connection'}</h3>
            <div className="form-grid">
              <label className="field">
                <span>Name</span>
                <input value={name} onChange={(event) => setName(event.target.value)} required />
              </label>
              <label className="field">
                <span>Type</span>
                <select
                  value={kind}
                  onChange={(event) => setKind(event.target.value as ProviderProfile['kind'])}
                >
                  <option value="ollama">Ollama</option>
                  <option value="openai-compatible">OpenAI-compatible</option>
                  <option value="llama-cpp">llama.cpp-compatible</option>
                </select>
              </label>
              <label className="field wide">
                <span>Base URL</span>
                <input
                  value={baseUrl}
                  onChange={(event) => setBaseUrl(event.target.value)}
                  inputMode="url"
                  required
                />
                <small>
                  <span>Prompts would go to:</span> <LocationBadge location={location} />
                </small>
              </label>
              <label className="field wide">
                <span>API key (optional)</span>
                <input
                  type="password"
                  value={apiKey}
                  autoComplete="off"
                  onChange={(event) => setApiKey(event.target.value)}
                  placeholder={editing?.apiKeyRef ? 'Saved — leave blank to keep it' : 'Optional'}
                />
                <small>Stored in the platform’s secure vault, never in Juniper’s database.</small>
              </label>
            </div>
            <div className="button-row">
              <button type="submit" className="button primary">
                {editing ? 'Save changes' : 'Add connection'}
              </button>
              <button type="button" className="button ghost" onClick={() => setFormOpen(false)}>
                Cancel
              </button>
            </div>
          </form>
        )}
        {providers.length === 0 && !formOpen ? (
          <EmptyState title="No connections yet">
            Models you download in Models run with Juniper’s own local engine and need no
            connection.
          </EmptyState>
        ) : (
          <ul className="provider-list">
            {providers.map((provider) => (
              <li key={provider.id} className="provider-row">
                <div className="provider-row-main">
                  <div className="provider-row-title">
                    <strong>{provider.name}</strong>
                    <LocationBadge location={provider.transportLocation} />
                  </div>
                  <small className="mono">{provider.baseUrl}</small>
                  <small>
                    {provider.enabled
                      ? provider.status === 'connected'
                        ? 'Connected'
                        : provider.status === 'offline'
                          ? 'Offline'
                          : 'Connection not checked'
                      : 'Disabled'}
                  </small>
                </div>
                <div className="provider-row-actions">
                  <button
                    className="button ghost"
                    onClick={() => void test(provider)}
                    disabled={checking === provider.id}
                  >
                    {checking === provider.id ? 'Checking…' : 'Test'}
                  </button>
                  <button className="button ghost" onClick={() => openForm(provider)}>
                    Edit
                  </button>
                  <button
                    className="button ghost"
                    onClick={() =>
                      update((current) => ({
                        ...current,
                        providers: current.providers.map((item) =>
                          item.id === provider.id ? { ...item, enabled: !item.enabled } : item,
                        ),
                      }))
                    }
                  >
                    {provider.enabled ? 'Disable' : 'Enable'}
                  </button>
                  <button
                    className="icon-button"
                    aria-label={`Remove ${provider.name}`}
                    title="Remove"
                    onClick={() => void remove(provider)}
                  >
                    <Icon name="trash" size={18} />
                  </button>
                </div>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section title="Device Link" description="Preview policy only — transport is not enabled.">
        <p className="muted">
          This release includes non-networked protocol and authorization policy for future Device
          Link work. It has no listener, discovery, usable pairing flow, peer connection, remote
          control, or Juniper Network transport. Private chats and host context remain on this
          device.
        </p>
        <div className="setting-row">
          <div className="setting-text">
            <span className="label">Device Link unavailable</span>
            <small>
              Pairing and scope controls stay disabled until the transport identity design is
              complete and verified.
            </small>
          </div>
          <div className="setting-control">
            <button className="button secondary" disabled>
              Preview only
            </button>
          </div>
        </div>
      </Section>
    </>
  )
}

export function ModelsRuntimeSettings({ data, update }: { data: AppData; update: Update }) {
  const external = useExternalModels(data, update)
  const [device, setDevice] = useState<DeviceCapabilities | null>(null)
  const [reference, setReference] = useState('')
  const hasOllama = data.providers.some(
    (provider) => provider.kind === 'ollama' && provider.enabled,
  )

  useEffect(() => {
    let active = true
    void getDeviceCapabilities()
      .then((next) => {
        if (active) setDevice(next)
      })
      .catch(() => undefined)
    return () => {
      active = false
    }
  }, [])

  return (
    <>
      <Section title="Local engine" description="Juniper’s own runtime for downloaded models.">
        <dl className="detail-list">
          <div>
            <dt>Status</dt>
            <dd>{nativeRuntimeLabel(device)}</dd>
          </div>
          <div>
            <dt>Memory</dt>
            <dd>
              {formatBytes(device?.availableMemoryBytes ?? device?.totalMemoryBytes)}
              {device?.memoryPressure && device.memoryPressure !== 'unknown'
                ? ` · ${device.memoryPressure} pressure`
                : ''}
            </dd>
          </div>
          <div>
            <dt>Free storage</dt>
            <dd>{formatBytes(device?.freeStorageBytes)}</dd>
          </div>
          <div>
            <dt>Processor</dt>
            <dd>{device ? `${device.architecture} · ${device.logicalCores} cores` : 'Unknown'}</dd>
          </div>
          <div>
            <dt>Model folder</dt>
            <dd className="mono">{device?.modelDirectory ?? 'Unknown'}</dd>
          </div>
        </dl>
        {device?.runtimes && device.runtimes.length > 0 && (
          <ul className="runtime-list">
            {device.runtimes.map((runtime) => (
              <li key={runtime.id}>
                <strong>{runtime.name}</strong>
                <span className={`chip maturity-${runtime.maturity}`}>{runtime.maturity}</span>
                <small>
                  {runtime.installed ? 'Installed' : 'Not installed'} · {runtime.state}
                  {runtime.reason ? ` · ${runtime.reason}` : ''}
                </small>
              </li>
            ))}
          </ul>
        )}
      </Section>

      <Section
        title="Download from Ollama"
        description="Pull a model by name through your Ollama connection."
      >
        <form
          className="inline-field"
          onSubmit={(event) => {
            event.preventDefault()
            void external.pullModel(reference).then((pulled) => {
              if (pulled) setReference('')
            })
          }}
        >
          <label className="field">
            <span>Model name</span>
            <input
              value={reference}
              onChange={(event) => setReference(event.target.value)}
              placeholder="e.g. qwen3:0.6b"
              maxLength={200}
            />
          </label>
          <button
            type="submit"
            className="button secondary"
            disabled={!reference.trim() || !hasOllama || external.pulling}
          >
            Download
          </button>
        </form>
        {!hasOllama && <p className="muted small">Add and enable an Ollama connection first.</p>}
        {external.pullStatus && (
          <div className="notice" role="status">
            <span>
              {external.pullStatus}
              {external.pullProgress.total
                ? ` · ${Math.round(((external.pullProgress.completed ?? 0) / external.pullProgress.total) * 100)}%`
                : ''}
            </span>
            {external.pulling && (
              <button className="link-button" onClick={external.cancelPull}>
                Cancel
              </button>
            )}
          </div>
        )}
      </Section>

      {!runningOnAndroid && (
        <Section
          title="Import a GGUF file"
          description="Desktop only. The picker validates and scopes the file, then Ollama imports it."
        >
          <div className="button-row">
            <button className="button secondary" onClick={() => void external.chooseGguf()}>
              {external.ggufSelection ? 'Choose another .gguf' : 'Choose .gguf file'}
            </button>
            {external.ggufSelection && (
              <span className="muted small">
                {external.ggufSelection.name} ·{' '}
                {(external.ggufSelection.sizeBytes / 1_000_000).toFixed(1)} MB
              </span>
            )}
          </div>
          {external.ggufSelection && (
            <div className="inline-field">
              <label className="field">
                <span>Name in Ollama</span>
                <input
                  value={external.ggufModelName}
                  onChange={(event) => external.setGgufModelName(event.target.value)}
                  maxLength={128}
                />
              </label>
              {external.importing ? (
                <button className="button secondary" onClick={external.cancelGguf}>
                  Cancel import
                </button>
              ) : (
                <button
                  className="button primary"
                  onClick={() => void external.importSelectedGguf()}
                  disabled={!external.ggufModelName.trim()}
                >
                  Import through Ollama
                </button>
              )}
            </div>
          )}
          {external.ggufStatus && (
            <p className="muted small" role="status">
              {external.ggufStatus}
            </p>
          )}
        </Section>
      )}

      <Section title="Fit guidance">
        <p className="muted">
          {external.hostMemory
            ? `Detected host memory: ${external.hostMemory}. Model fit still depends on quantization and runtime overhead.`
            : 'Host memory is unavailable here. Juniper will not guess whether a model fits.'}
        </p>
        <p className="muted small">
          GPU acceleration and throughput remain unknown unless the provider reports them.
        </p>
      </Section>
    </>
  )
}
