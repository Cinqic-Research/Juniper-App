import { useEffect, useState } from 'react'
import { getDeviceCapabilities, runningInTauri } from '../lib/runtime'
import { JuniperMark } from './branding'
import { Icon } from './icons'
import type { IconName } from './icons'
import { nativeRuntimeLabel } from './ModelsScreen'
import { Modal } from './overlays'
import { Segmented } from './ui'
import type { ThemePreference } from '../types'

export function Onboarding({
  open,
  onDone,
  theme,
  onTheme,
}: {
  open: boolean
  onDone: () => void
  theme: ThemePreference
  onTheme: (theme: ThemePreference) => void
}) {
  const [step, setStep] = useState(0)
  const [runtimeStatus, setRuntimeStatus] = useState('Checking this device…')

  useEffect(() => {
    if (!open) return
    setStep(0)
    if (!runningInTauri) {
      setRuntimeStatus(
        import.meta.env.DEV ? 'Browser development preview' : 'Native runtime unavailable',
      )
      return
    }
    void getDeviceCapabilities()
      .then((device) => setRuntimeStatus(`Juniper’s local engine: ${nativeRuntimeLabel(device)}`))
      .catch(() => setRuntimeStatus('Juniper could not inspect this device yet'))
  }, [open])

  const steps: Array<{ eyebrow: string; title: string; copy: string; art: 'logo' | IconName }> = [
    {
      eyebrow: 'Welcome to Juniper',
      title: 'Your AI. Your models. Your machine.',
      copy: 'A local-first place to chat with models you choose, shaped into assistants that feel like yours.',
      art: 'logo',
    },
    {
      eyebrow: 'Private by default',
      title: 'Nothing leaves unless you choose it.',
      copy: 'No account. Local chats stay local. Remote providers are clearly labelled whenever you opt in to one.',
      art: 'shield',
    },
    {
      eyebrow: 'Start with a model',
      title: 'Bring the intelligence you trust.',
      copy: `${runtimeStatus}. Download a model made for this device in Models, or connect one you already run.`,
      art: 'models',
    },
    {
      eyebrow: 'Agent (Beta)',
      title: 'Hand off work, and stay in control.',
      copy: 'Agent works through a goal with the tools and folders you allow, asks before changing anything, and shows every step. It is in beta and runs only while Juniper is open.',
      art: 'sparkle',
    },
    {
      eyebrow: 'Meet Juniper',
      title: 'A capable older sister for the things you’re figuring out.',
      copy: 'Warm, practical, direct, and honest about uncertainty. Change her personality — or build new assistants — in Settings.',
      art: 'person',
    },
  ]
  const current = steps[step]!
  const last = step === steps.length - 1

  return (
    <Modal open={open} onClose={onDone} title={current.title} hideTitle variant="dialog">
      <div className="onboarding">
        <div className="onboarding-art" aria-hidden="true">
          {current.art === 'logo' ? (
            <JuniperMark className="onboarding-logo" alt="" />
          ) : (
            <Icon name={current.art} size={40} />
          )}
        </div>
        <p className="eyebrow">{current.eyebrow}</p>
        <p className="onboarding-title" aria-hidden="true">
          {current.title}
        </p>
        <p className="onboarding-copy" aria-live="polite">
          {current.copy}
        </p>
        {step === 0 && (
          <div className="onboarding-theme">
            <Segmented<ThemePreference>
              label="Appearance"
              value={theme}
              onChange={onTheme}
              options={[
                { value: 'system', label: 'System' },
                { value: 'light', label: 'Light' },
                { value: 'dark', label: 'Dark' },
              ]}
            />
          </div>
        )}
        <div className="onboarding-progress" aria-label={`Step ${step + 1} of ${steps.length}`}>
          {steps.map((item, index) => (
            <span key={item.eyebrow} className={index <= step ? 'active' : ''} />
          ))}
        </div>
        <div className="onboarding-actions">
          <button
            className="button ghost"
            onClick={() => (step > 0 ? setStep((value) => value - 1) : onDone())}
          >
            {step > 0 ? 'Back' : 'Skip'}
          </button>
          {/* One button keeps keyboard focus in place as the steps advance. */}
          <button
            className="button primary"
            data-autofocus
            onClick={() => (last ? onDone() : setStep((value) => value + 1))}
          >
            {last ? 'Enter Juniper' : 'Continue'}
          </button>
        </div>
        <small className="onboarding-footnote">
          No account. No telemetry. You can replay this welcome from Settings › General.
        </small>
      </div>
    </Modal>
  )
}
