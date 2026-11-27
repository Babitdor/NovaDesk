import { useCallback, useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'

import {
  browserBounds,
  browserClose,
  browserHistory,
  browserLocalServers,
  browserReload,
  browserOpen,
  browserVisibility,
  errorText,
} from '../lib/ipc'

/// A bare port is what people type when they mean "the server I just started";
/// anything else keeps its own shape so a path or a query is not discarded.
function normalize(input: string): string {
  const text = input.trim()
  if (!text) return ''
  if (/^\d{2,5}$/.test(text)) return 'http://localhost:' + text
  if (/^(?:[^\s/:]+|\[[\da-f:]+\]):\d{2,5}(?:[/?#]|$)/i.test(text)) return 'http://' + text
  if (/^https?:\/\//i.test(text)) return text
  return 'https://' + text
}

/// The Browser tab is a real WebView2 child of the window, created by
/// `browser_open` and placed over `.browser-stage`. It paints above the page and
/// ignores the DOM, so this component's whole job is to keep its rectangle and
/// its visibility honest.
export type BrowserTarget = { url: string; settle: (opened: Promise<string>) => void }

export function BrowserPane({ active, target }: {
  active: boolean
  target?: BrowserTarget | null
}) {
  const host = useRef<HTMLDivElement>(null)
  const [url, setUrl] = useState('')
  const [loaded, setLoaded] = useState<string | null>(null)
  const [found, setFound] = useState<string[] | null>(null)
  const [notice, setNotice] = useState<string | null>(null)

  useEffect(() => {
    let disposed = false
    const listener = listen<string>('browser-location', ({ payload }) => {
      if (!disposed) setUrl(payload)
    })
    void listener.catch(() => {})
    return () => {
      disposed = true
      void listener.then((unlisten) => unlisten()).catch(() => {})
    }
  }, [])

  const box = useCallback(() => {
    const rect = host.current?.getBoundingClientRect()
    if (!rect || rect.width === 0 || rect.height === 0) return null
    return { x: rect.left, y: rect.top, w: rect.width, h: rect.height }
  }, [])

  const open = useCallback(
    async (raw: string) => {
      const target = normalize(raw)
      if (!target) throw new Error('Enter a browser URL.')
      const at = box()
      if (!at) {
        throw new Error('The browser has no room to sit in yet.')
      }

      await browserOpen(target, at.x, at.y, at.w, at.h)
      setUrl(target)
      setLoaded(target)
      setNotice(null)
      return target
    },
    [box],
  )

  // A tool's open_browser lands here: the pane owns the rectangle the native
  // child is placed in, so it is the only component that can honour the request.
  useEffect(() => {
    if (!target) return
    const pending = setTimeout(() => target.settle(open(target.url)), 0)
    return () => clearTimeout(pending)
  }, [target, open])

  const navigate = (address: string) => void open(address).catch((err) => setNotice(errorText(err)))

  const close = useCallback(async () => {
    try {
      await browserClose()
      setFound(null)
      setLoaded(null)
    } catch (err) {
      setNotice(errorText(err))
    }
  }, [])

  const refresh = () => {
    setNotice(null)
    setFound(null)
  }

  useEffect(() => {
    if (!active || loaded || found !== null) return
    let stale = false
    void browserLocalServers().then(
      (servers) => { if (!stale) setFound(servers) },
      (err) => {
        if (stale) return
        setFound([])
        setNotice(errorText(err))
      },
    )
    return () => { stale = true }
  }, [active, loaded, found])

  // The tab strip hides panes with CSS; a native child needs telling, or it keeps
  // painting over whatever tab is actually open.
  useEffect(() => {
    if (!loaded) return
    if (!active) {
      void browserVisibility(false).catch(() => {})
      return
    }
    const at = box()
    if (!at) return
    let stale = false
    void browserBounds(at.x, at.y, at.w, at.h).then(() => {
      if (!stale) return browserVisibility(true)
    }).catch(() => {})
    return () => { stale = true }
  }, [active, box, loaded])

  // A resize moves the pane without moving any DOM the child knows about.
  useEffect(() => {
    const element = host.current
    if (!element || !loaded) return

    const syncBounds = () => {
      if (!active) return
      const at = box()
      if (at) void browserBounds(at.x, at.y, at.w, at.h).catch(() => {})
    }
    const observer = new ResizeObserver(syncBounds)
    observer.observe(element)
    window.addEventListener('resize', syncBounds)
    return () => {
      observer.disconnect()
      window.removeEventListener('resize', syncBounds)
    }
  }, [active, box, loaded])

  const go = (direction: 'back' | 'forward') =>
    void browserHistory(direction).catch((err) => setNotice(errorText(err)))

  const reload = () => {
    if (loaded) void browserReload().catch((err) => setNotice(errorText(err)))
  }

  return (
    <section className="browser">
      <div className="browser-bar">
        <button
          type="button"
          className="ghost"
          aria-label="Back"
          disabled={!loaded}
          onClick={() => go('back')}
        >
          <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
            <path d="M10 3 5 8l5 5" fill="none" stroke="currentColor" strokeWidth="1.6" />
          </svg>
        </button>
        <button
          type="button"
          className="ghost"
          aria-label="Forward"
          disabled={!loaded}
          onClick={() => go('forward')}
        >
          <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
            <path d="M6 3l5 5-5 5" fill="none" stroke="currentColor" strokeWidth="1.6" />
          </svg>
        </button>
        <button
          type="button"
          className="ghost"
          aria-label="Reload"
          disabled={!loaded}
          onClick={reload}
        >
          <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
            <path
              d="M12.5 8a4.5 4.5 0 1 1-1.3-3.2"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.6"
            />
            <path d="M12.8 2v3h-3" fill="none" stroke="currentColor" strokeWidth="1.6" />
          </svg>
        </button>

        <div className="url-field">
          <input
            className="browser-url"
            value={url}
            placeholder="Enter URL or localhost port"
            aria-label="Address"
            onChange={(event) => setUrl(event.target.value)}
            onKeyDown={(event) => {
              if (event.key !== 'Enter') return
              event.preventDefault()
              navigate(url)
            }}
          />
          <button
            type="button"
            className="ghost url-go"
            aria-label="Open this address"
            onClick={() => navigate(url)}
          >
            <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
              <path d="M2 8h11M9 4l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.6" />
            </svg>
          </button>
        </div>

        {loaded && (
          <button type="button" className="ghost" aria-label="Close page" onClick={() => void close()}>
            ×
          </button>
        )}
      </div>

      {notice && <p className="panel-error">{notice}</p>}

      <div className="browser-content">
        {!loaded && (
          <div className="browser-home">
            <div className="section-head">
              <span>Local TCP listeners</span>
              <button
                type="button"
                className="ghost"
                aria-label="Look for local servers again"
                onClick={refresh}
                disabled={found === null}
              >
                <svg width="13" height="13" viewBox="0 0 16 16" aria-hidden="true">
                  <path
                    d="M12.5 8a4.5 4.5 0 1 1-1.3-3.2"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.6"
                  />
                  <path d="M12.8 2v3h-3" fill="none" stroke="currentColor" strokeWidth="1.6" />
                </svg>
              </button>
            </div>

            <ul className="server-list">
              {found === null && <li className="server-empty">Looking for local servers…</li>}
              {found?.length === 0 && !notice && (
                <li className="server-empty">
                  No local servers found. Start your app in the terminal, then refresh.
                </li>
              )}
              {(found ?? []).map((address) => (
                <li key={address}>
                  <button
                    type="button"
                    className="server-row"
                    aria-label={'Open ' + address}
                    onClick={() => navigate('http://' + address)}
                  >
                    <span className="server-dot" aria-hidden="true" />
                    <span className="server-name">{address}</span>
                    <span className="server-chevron" aria-hidden="true">
                      ›
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}

        {/* The child webview covers exactly this rect, so nothing drawn inside it
            is ever seen while a page is loaded. */}
        <div className="browser-stage" ref={host} data-loaded={!!loaded} />
      </div>
    </section>
  )
}
