import { useEffect, useRef, useState } from 'react'

type Citation = { document_id: string; chunk_id: string; page?: number; score: number; snippet: string }
type QueryResp = { answer: string; citations: Citation[]; contexts: string[]; mode: string; latency_ms: number }

export default function Home() {
  const [q, setQ] = useState('你好，平台是什么？')
  const [answer, setAnswer] = useState('')
  const [contexts, setContexts] = useState<string[]>([])
  const [cites, setCites] = useState<Citation[]>([])
  const esRef = useRef<EventSource | null>(null)
  const [sessionId, setSessionId] = useState<string | null>(null)
  const [events, setEvents] = useState<string[]>([])
  const [pendingTool, setPendingTool] = useState<string | null>(null)
  const [toolInput, setToolInput] = useState<string>('')

  async function ask() {
    const resp = await fetch('/api/v1/query', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ query: q, top_k: 3, mode: 'rag' }) })
    const data: QueryResp = await resp.json()
    setAnswer(data.answer)
    setContexts(data.contexts || [])
    setCites(data.citations || [])
    setEvents((e)=>["[non-stream] completed", ...e])
  }

  async function askStream() {
    setAnswer('')
    esRef.current?.close()
    // 直接 GET SSE（后端已支持 GET）
    const es = new EventSource(`/api/v1/query/stream?query=${encodeURIComponent(q)}&top_k=3`)
    es.onmessage = (e) => { setAnswer((a) => a + e.data); setEvents((ev)=>[e.data, ...ev]) }
    es.addEventListener('text', (e) => { const d=(e as MessageEvent).data; setAnswer((a) => a + d); setEvents((ev)=>[`[text] ${d}`, ...ev]) })
    es.addEventListener('reasoning', (e) => { const d=(e as MessageEvent).data; setAnswer((a) => a + '\n[thinking]' + d + '\n'); setEvents((ev)=>[`[reasoning] ${d}`, ...ev]) })
    es.addEventListener('tool_call', (e) => { const d=(e as MessageEvent).data; setAnswer((a) => a + `\n[tool_call] ${d}\n`); setEvents((ev)=>[`[tool_call] ${d}`, ...ev]) })
    es.addEventListener('tool_result', (e) => { const d=(e as MessageEvent).data; setAnswer((a) => a + `\n[tool_result] ${d}\n`); setEvents((ev)=>[`[tool_result] ${d}`, ...ev]) })
    es.addEventListener('final', (e) => es.close())
    es.addEventListener('error', (e) => es.close())
    esRef.current = es
  }

  // 客户端驱动的工具闭环演示：
  async function askClientLoop() {
    setAnswer('')
    esRef.current?.close()
    // 1) 创建会话
    const r = await fetch('/api/v1/session/start', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ query: q, top_k: 3 }) })
    const { session_id } = await r.json()
    setSessionId(session_id)
    // 2) 开始拉流（递归继续直到 final）
    await streamSession(session_id)
  }

  async function streamSession(id: string): Promise<void> {
    const es = new EventSource(`/api/v1/session/stream?session_id=${id}`)
    esRef.current = es
    const done = new Promise<void>((resolve) => {
      es.addEventListener('text', (e) => { const d=(e as MessageEvent).data; setAnswer((a)=>a + d); setEvents((ev)=>[`[text] ${d}`, ...ev]) })
      es.addEventListener('reasoning', (e) => { const d=(e as MessageEvent).data; setAnswer((a)=>a + `\n[thinking] ${d}\n`); setEvents((ev)=>[`[reasoning] ${d}`, ...ev]) })
      es.addEventListener('tool_call', async (e) => {
        const payload = (e as MessageEvent).data as string
        setPendingTool(payload)
        setToolInput(payload.startsWith('time_now') ? new Date().toISOString() : `client_result_for: ${payload}`)
        setEvents((ev)=>[`[tool_call] ${payload}`, ...ev])
        es.close()
        resolve()
      })
      es.addEventListener('final', (_e) => { es.close(); resolve() })
      es.addEventListener('error', (_e) => { es.close(); resolve() })
    })
    await done
  }

  async function submitToolResult() {
    if (!sessionId || !toolInput) return
    
    try {
      await fetch('/api/v1/session/tool_result', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ session_id: sessionId, result: toolInput })
      })
      
      setPendingTool(null)
      setToolInput('')
      setEvents((ev) => [`[tool_result] ${toolInput}`, ...ev])
      
      // 继续拉流
      await streamSession(sessionId)
    } catch (error) {
      console.error('Failed to submit tool result:', error)
      setEvents((ev) => [`[error] Failed to submit tool result: ${error}`, ...ev])
    }
  }

  useEffect(() => () => esRef.current?.close(), [])

  return (
    <main style={{ maxWidth: 1200, margin: '24px auto', fontFamily: 'system-ui, -apple-system, Segoe UI, Roboto, Helvetica, Arial' }}>
      <h3>KB RAG Chat (Next.js)</h3>
      <div style={{ display: 'flex', gap: 8 }}>
        <input value={q} onChange={(e) => setQ(e.target.value)} style={{ flex: 1, padding: 8 }} placeholder="输入你的问题..." />
        <button onClick={ask}>Query</button>
        <button onClick={askStream}>SSE Stream</button>
        <button onClick={askClientLoop}>Client Tool Loop</button>
      </div>
      <h4>Answer</h4>
      <pre style={{ whiteSpace: 'pre-wrap', border: '1px solid #ddd', padding: 12, borderRadius: 8, minHeight: 120 }}
        dangerouslySetInnerHTML={{ __html: answer.replace(/\[(\d+)\]/g, (_m, g1) => `<a href="#ctx-${g1}">[${g1}]</a>`) }} />
      <h4>Contexts</h4>
      {(contexts || []).map((t, i) => (
        <div id={`ctx-${i+1}`} key={i} style={{ background: '#f7f7ff', borderLeft: '3px solid #88f', margin: '8px 0', padding: 8 }}>
          <b>[{i + 1}]</b> {t}
        </div>
      ))}
      <h4>Citations</h4>
      {(cites || []).map((c, idx) => (
        <div key={idx} style={{ fontSize: 12, color: '#555' }}>
          {c.document_id}#{c.chunk_id} p={c.page ?? '-'} score={c.score.toFixed(3)}: {c.snippet}
        </div>
      ))}
      {pendingTool && (
        <div style={{ marginTop: 12, padding: 12, border: '1px dashed #88f' }}>
          <div style={{ marginBottom: 6 }}>工具待执行：{pendingTool}</div>
          <textarea value={toolInput} onChange={e=>setToolInput(e.target.value)} rows={3} style={{ width: '100%', padding: 8 }} />
          <div style={{ marginTop: 8 }}>
            <button onClick={submitToolResult}>提交工具结果</button>
          </div>
        </div>
      )}
      <h4>Tool Events</h4>
      <div style={{ maxHeight: 180, overflow: 'auto', border: '1px solid #eee', padding: 8, fontSize: 12 }}>
        {events.map((e, idx) => (
          <div key={idx}>• {e}</div>
        ))}
      </div>
    </main>
  )
}
