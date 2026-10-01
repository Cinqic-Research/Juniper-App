import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './app/App'
import { StartupErrorBoundary } from './app/StartupErrorBoundary'
import { installFlexGapFallback } from './lib/flex-gap-fallback'
import '@fontsource-variable/inter/wght.css'
import '@fontsource-variable/atkinson-hyperlegible-next/wght.css'
import '@fontsource/opendyslexic/400.css'
import '@fontsource/opendyslexic/700.css'
import './styles/tokens.css'
import './styles/app.css'

try {
  installFlexGapFallback()
  const root = document.getElementById('root')
  if (!root) throw new Error('Root element missing')
  createRoot(root).render(
    <StrictMode>
      <StartupErrorBoundary>
        <App />
      </StartupErrorBoundary>
    </StrictMode>,
  )
} catch {
  const report = (window as Window & { __juniperBootstrapFailed?: (kind: string) => void })
    .__juniperBootstrapFailed
  report?.('mount error')
}
