import React, { useMemo, useRef, useState } from 'react'
import { ClipboardCheck, Download, RefreshCw, Save, Sparkles, X } from 'lucide-react'
import { exportTrajectoryReviews } from '../api/client'
import { useAssistTrajectoryReview, useSaveTrajectoryReview, useTrajectories, useTrajectory } from '../api/hooks'
import type { AgentEvent, ReviewAssistSuggestion, SaveTrajectoryReviewRequest, TrajectorySummary } from '../api/types'

const labelOptions: SaveTrajectoryReviewRequest['overall_label'][] = ['good', 'bad', 'needs_review']
const failureTypes = ['bad_answer', 'bad_tool_use', 'hallucination', 'inefficient', 'unsafe', 'other']

const fmtTime = (ts?: string) => ts ? new Date(ts).toLocaleString([], { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '-'

interface TrajectoryTurn {
  index: number
  request?: AgentEvent
  response?: AgentEvent
}

interface TimelineStep {
  index: number
  kind: 'model' | 'tool'
  request?: AgentEvent
  response?: AgentEvent
  toolCall?: Record<string, unknown>
  toolResult?: Record<string, unknown>
  eventId: string
}

function getPath(value: unknown, path: Array<string | number>): unknown {
  let current = value
  for (const key of path) {
    if (current == null || typeof current !== 'object') return undefined
    current = (current as Record<string, unknown>)[key]
  }
  return current
}

function compactText(value: unknown): string {
  if (typeof value === 'string') return value.trim()
  if (Array.isArray(value)) {
    return value.map(v => compactText(v)).filter(Boolean).join('\n')
  }
  if (value && typeof value === 'object') {
    return JSON.stringify(value)
  }
  return ''
}

function readableContent(ev: AgentEvent): { title: string; body: string; meta: string[] } {
  const payload = ev.payload
  const request = payload?.request
  const response = payload?.response

  if (request) {
    const messages = getPath(request, ['messages'])
    const lastUser = Array.isArray(messages)
      ? [...messages].reverse().find(m => (m as Record<string, unknown>).role === 'user')
      : undefined
    const prompt = compactText(lastUser ? (lastUser as Record<string, unknown>).content : messages)
    return {
      title: 'Agent asked the model',
      body: prompt || 'Request captured, but prompt text was not found.',
      meta: [
        `model: ${compactText(getPath(request, ['model'])) || ev.model || 'unknown'}`,
        `max tokens: ${compactText(getPath(request, ['max_tokens'])) || 'n/a'}`,
      ],
    }
  }

  if (response) {
    const content = getPath(response, ['choices', 0, 'message', 'content'])
    const toolCalls = getPath(response, ['choices', 0, 'message', 'tool_calls'])
    const finish = getPath(response, ['choices', 0, 'finish_reason'])
    const readableToolCalls = Array.isArray(toolCalls)
      ? toolCalls.map(call => {
        const fn = (call as Record<string, unknown>).function as Record<string, unknown> | undefined
        const name = compactText(fn?.name || 'tool')
        const args = summarizeToolArgs(name, fn?.arguments)
        return `${name}: ${args}`
      }).join('\n')
      : ''
      return {
      title: 'Model responded',
      body: compactText(content) || readableToolCalls || 'Response captured, but answer text was not found.',
      meta: [
        `finish: ${compactText(finish) || ev.finish_reason || 'n/a'}`,
        `tokens: ${ev.total_tokens ?? compactText(getPath(response, ['usage', 'total_tokens'])) ?? 0}`,
      ],
    }
  }

  return {
    title: ev.direction === 'outbound' ? 'Agent request' : ev.direction === 'inbound' ? 'Model response' : 'Event',
    body: 'No readable payload captured for this step.',
    meta: [`raw size: ${ev.raw_size_bytes ?? 'n/a'} bytes`],
  }
}

function turnContent(turn: TrajectoryTurn) {
  const request = turn.request ? readableContent(turn.request) : null
  const response = turn.response ? readableContent(turn.response) : null
  return {
    prompt: request?.body || 'No prompt captured.',
    answer: response?.body || 'No response captured.',
    meta: [
      ...(request?.meta ?? []),
      ...(response?.meta ?? []),
    ],
  }
}

function toolCallKey(call: Record<string, unknown>): string {
  const id = call.id
  if (typeof id === 'string' && id) return id
  const fn = call.function as Record<string, unknown> | undefined
  return typeof fn?.name === 'string' ? fn.name : 'tool'
}

function responseToolCalls(ev?: AgentEvent): Record<string, unknown>[] {
  const calls = getPath(ev?.payload?.response, ['choices', 0, 'message', 'tool_calls'])
  return Array.isArray(calls) ? calls as Record<string, unknown>[] : []
}

function requestToolResults(ev?: AgentEvent): Record<string, unknown>[] {
  const messages = getPath(ev?.payload?.request, ['messages'])
  if (!Array.isArray(messages)) return []
  return messages.filter(message => (message as Record<string, unknown>).role === 'tool') as Record<string, unknown>[]
}

function formatJsonMaybe(value: unknown): string {
  if (typeof value !== 'string') return compactText(value)
  try {
    return JSON.stringify(JSON.parse(value), null, 2)
  } catch {
    return value
  }
}

function parseJsonString(value: unknown): unknown {
  if (typeof value !== 'string') return value
  try {
    return JSON.parse(value)
  } catch {
    return value
  }
}

function summarizeToolArgs(name: string, rawArgs: unknown): string {
  const args = parseJsonString(rawArgs)
  if (name === 'divide_number' && args && typeof args === 'object') {
    const data = args as Record<string, unknown>
    const divisors = Array.isArray(data.divisors) ? data.divisors.join(', ') : compactText(data.divisors)
    return `number=${compactText(data.number)}, divisors=${divisors}`
  }
  return formatJsonMaybe(rawArgs)
}

function summarizeToolResult(name: string, rawResult: unknown): string {
  const result = parseJsonString(rawResult)
  if (name === 'divide_number' && result && typeof result === 'object') {
    const rows = (result as Record<string, unknown>).results
    if (Array.isArray(rows)) {
      return rows.map(row => {
        const data = row as Record<string, unknown>
        const divisible = data.divides_evenly ? 'divides evenly' : `remainder ${data.remainder}`
        return `${data.number} / ${data.divisor} = ${data.quotient} (${divisible})`
      }).join('\n')
    }
  }
  return formatJsonMaybe(rawResult)
}

function toolStepContent(step: TimelineStep) {
  const fn = step.toolCall?.function as Record<string, unknown> | undefined
  const name = compactText(fn?.name || step.toolResult?.name || 'tool')
  const args = summarizeToolArgs(name, fn?.arguments)
  const result = summarizeToolResult(name, step.toolResult?.content)
  return {
    name,
    args: args || 'No tool arguments captured.',
    result: result || 'No tool result captured yet.',
  }
}

function groupTurns(events: AgentEvent[]): TrajectoryTurn[] {
  const turns: TrajectoryTurn[] = []
  let current: TrajectoryTurn | null = null

  events.forEach(ev => {
    if (ev.direction === 'outbound') {
      current = { index: turns.length + 1, request: ev }
      turns.push(current)
      return
    }

    if (ev.direction === 'inbound') {
      if (!current || current.response) {
        current = { index: turns.length + 1 }
        turns.push(current)
      }
      current.response = ev
      return
    }

    turns.push({ index: turns.length + 1, request: ev })
  })

  return turns
}

function buildTimeline(events: AgentEvent[]): TimelineStep[] {
  const turns = groupTurns(events)
  const steps: TimelineStep[] = []

  turns.forEach((turn, turnIndex) => {
    const modelEventId = turn.response?.id || turn.request?.id || ''
    steps.push({
      index: steps.length + 1,
      kind: 'model',
      request: turn.request,
      response: turn.response,
      eventId: modelEventId,
    })

    const nextTurn = turns[turnIndex + 1]
    const toolResults = requestToolResults(nextTurn?.request)
    responseToolCalls(turn.response).forEach(call => {
      const key = toolCallKey(call)
      const result = toolResults.find(tool => tool.tool_call_id === key || tool.name === key)
      steps.push({
        index: steps.length + 1,
        kind: 'tool',
        toolCall: call,
        toolResult: result,
        eventId: nextTurn?.request?.id || turn.response?.id || '',
      })
    })
  })

  return steps
}

function stepLabel(step: TimelineStep | undefined): string {
  if (!step) return 'No matching step'
  if (step.kind === 'tool') return `Step ${step.index}: tool ${toolStepContent(step).name}`
  return `Step ${step.index}: model ${step.request?.model || step.response?.model || 'unknown'}`
}

function findStepByEventId(eventId: string | null | undefined, steps: TimelineStep[]): TimelineStep | undefined {
  if (!eventId) return undefined
  return steps.find(step => step.eventId === eventId)
}

function findStepByIndex(index: number | null | undefined, steps: TimelineStep[]): TimelineStep | undefined {
  if (!index) return undefined
  return steps.find(step => step.index === index)
}

function screeningBadges(s: TrajectorySummary): string[] {
  const badges = [s.overall_label ? 'reviewed' : 'unreviewed', `${s.event_count} events`]
  if ((s.event_count ?? 0) >= 8) badges.push('multi-step')
  if ((s.total_tokens ?? 0) >= 500) badges.push('high tokens')
  if ((s.total_cost_usd ?? 0) >= 0.0001) badges.push('cost spike')
  if (s.failure_type) badges.push(s.failure_type.split('_').join(' '))
  return badges
}

function payloadText(ev: AgentEvent): string {
  const payload = ev.payload
  if (!payload) return 'No captured payload'
  const value = payload.request ?? payload.response ?? payload
  return JSON.stringify(value, null, 2)
}

function labelClass(label: string | null | undefined): string {
  if (label === 'good') return 'badge badge-success'
  if (label === 'bad') return 'badge badge-danger'
  if (label === 'needs_review') return 'badge badge-warning'
  return 'badge badge-neutral'
}

function SessionList({
  sessions,
  selected,
  onSelect,
}: {
  sessions: TrajectorySummary[]
  selected: string
  onSelect: (id: string) => void
}) {
  return (
    <div className="bg-white border border-slate-200 rounded-xl overflow-hidden">
      <div className="px-4 py-3 border-b border-slate-100">
        <p className="text-xs font-bold uppercase tracking-[0.12em] text-slate-400">Captured Trajectories</p>
      </div>
      <div className="divide-y divide-slate-100 max-h-[680px] overflow-y-auto">
        {sessions.map(s => (
          <button
            key={s.session_id}
            onClick={() => onSelect(s.session_id)}
            className={`w-full text-left px-4 py-3 hover:bg-slate-50 transition-colors ${selected === s.session_id ? 'bg-primary/5 border-l-2 border-primary' : ''}`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-sm font-semibold text-slate-800 truncate">{s.agent_id || 'unknown-agent'}</span>
              <span className={labelClass(s.overall_label)}>{s.overall_label ?? 'unreviewed'}</span>
            </div>
            <div className="mt-2 text-xs text-slate-500 space-y-1">
              <p className="metric-font truncate">{s.session_id}</p>
              <p>{fmtTime(s.started_at)} · {s.event_count} steps · {s.model || 'unknown model'}</p>
              <p>{s.total_tokens?.toLocaleString?.() ?? 0} tokens · ${Number(s.total_cost_usd ?? 0).toFixed(5)}</p>
            </div>
            <div className="mt-2 flex flex-wrap gap-1.5">
              {screeningBadges(s).map(badge => (
                <span key={badge} className="badge badge-neutral">{badge}</span>
              ))}
            </div>
          </button>
        ))}
        {sessions.length === 0 && (
          <div className="px-4 py-10 text-center text-slate-400">
            <ClipboardCheck className="w-8 h-8 mx-auto mb-2 text-slate-300" />
            <p className="text-sm font-medium">No trajectories yet</p>
          </div>
        )}
      </div>
    </div>
  )
}

export default function ReviewPage() {
  const { data, isLoading, refetch } = useTrajectories()
  const sessions = (data?.data ?? []).filter(session =>
    session.agent_id !== 'unknown' ||
    (session.total_tokens ?? 0) > 0 ||
    (session.total_cost_usd ?? 0) > 0 ||
    session.model,
  )
  const [selectedSession, setSelectedSession] = useState('')
  const [detailOpen, setDetailOpen] = useState(false)
  const activeSession = selectedSession || sessions[0]?.session_id || ''
  const { data: detailData } = useTrajectory(activeSession)
  const saveReview = useSaveTrajectoryReview()
  const assistReview = useAssistTrajectoryReview()

  const detail = detailData?.data
  const events = detail?.events ?? []
  const timelineSteps = useMemo(() => buildTimeline(events), [events])
  const activeSummary = sessions.find(s => s.session_id === activeSession)
  const [overallLabel, setOverallLabel] = useState<SaveTrajectoryReviewRequest['overall_label']>('needs_review')
  const [failureType, setFailureType] = useState('bad_answer')
  const [failureEventId, setFailureEventId] = useState('')
  const [notes, setNotes] = useState('')
  const [timelineMode, setTimelineMode] = useState<'readable' | 'raw'>('readable')
  const [assistSuggestion, setAssistSuggestion] = useState<ReviewAssistSuggestion | null>(null)
  const timelineRefs = useRef<Record<string, HTMLDivElement | null>>({})

  React.useEffect(() => {
    if (!detail?.review) return
    setOverallLabel(detail.review.overall_label)
    setFailureType(detail.review.failure_type || 'bad_answer')
    setFailureEventId(detail.review.failure_event_id || '')
    setNotes(detail.review.notes || '')
  }, [detail?.review?.id, activeSession])

  React.useEffect(() => {
    setAssistSuggestion(null)
  }, [activeSession])

  const selectedStep = useMemo(() => events.find(e => e.id === failureEventId), [events, failureEventId])
  const assistantStep = useMemo(
    () => findStepByEventId(assistSuggestion?.failure_event_id, timelineSteps) || findStepByIndex(assistSuggestion?.failure_step_index, timelineSteps),
    [assistSuggestion?.failure_event_id, assistSuggestion?.failure_step_index, timelineSteps],
  )
  const applyAssistSuggestion = (suggestion: ReviewAssistSuggestion) => {
    const suggestedStep = findStepByEventId(suggestion.failure_event_id, timelineSteps) || findStepByIndex(suggestion.failure_step_index, timelineSteps)
    setOverallLabel(suggestion.suggested_label)
    setFailureType(suggestion.failure_type || 'other')
    setFailureEventId(suggestion.failure_event_id || suggestedStep?.eventId || '')
    setNotes(suggestion.critique)
  }
  const save = () => {
    if (!activeSession) return
    saveReview.mutate({
      sessionId: activeSession,
      body: {
        reviewer: 'labelbox-demo-reviewer',
        overall_label: overallLabel,
        failure_type: overallLabel === 'good' ? null : failureType,
        failure_event_id: overallLabel === 'good' ? null : failureEventId || null,
        notes,
      },
    })
  }

  const assist = () => {
    if (!activeSession) return
    assistReview.mutate(activeSession, {
      onSuccess: (resp) => {
        setAssistSuggestion(resp.data)
        applyAssistSuggestion(resp.data)
      },
    })
  }

  const exportJsonl = async () => {
    const text = await exportTrajectoryReviews()
    const blob = new Blob([text], { type: 'application/x-ndjson' })
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'langtrail-trajectory-reviews.jsonl'
    a.click()
    URL.revokeObjectURL(url)
  }

  const selectTrajectory = (id: string) => {
    setSelectedSession(id)
    setDetailOpen(true)
  }

  return (
    <div className="flex-1 overflow-y-auto p-4 sm:p-6">
      <div className="max-w-[1500px] mx-auto space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h2 className="text-lg font-bold text-slate-900">Trajectory Review</h2>
            <p className="text-xs text-slate-400">Turn captured agent runs into human-labeled evaluation data</p>
          </div>
          <div className="flex items-center gap-2">
            <button onClick={() => refetch()} className="btn-secondary flex items-center gap-1.5 text-xs">
              <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`} />
              Refresh
            </button>
            <button onClick={exportJsonl} className="btn-secondary flex items-center gap-1.5 text-xs">
              <Download className="w-3.5 h-3.5" />
              Export JSONL
            </button>
          </div>
        </div>

        <div className="grid grid-cols-1 xl:grid-cols-[360px_minmax(0,1fr)] gap-4">
          <SessionList sessions={sessions} selected={activeSession} onSelect={selectTrajectory} />

          <div className={`space-y-4 fixed inset-0 z-50 overflow-y-auto bg-slate-50 p-4 xl:static xl:z-auto xl:overflow-visible xl:bg-transparent xl:p-0 ${detailOpen ? 'block' : 'hidden xl:block'}`}>
            <div className="xl:hidden flex items-center justify-between rounded-xl border border-slate-200 bg-white px-4 py-3">
              <div className="min-w-0">
                <p className="text-sm font-semibold text-slate-900 truncate">{activeSummary?.agent_id || 'Trajectory Review'}</p>
                <p className="text-xs text-slate-400 metric-font truncate">{activeSession || 'No trajectory selected'}</p>
              </div>
              <button
                onClick={() => setDetailOpen(false)}
                className="w-8 h-8 rounded-lg border border-slate-200 flex items-center justify-center text-slate-500"
                aria-label="Close trajectory review"
                title="Close"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <div className="bg-white border border-slate-200 rounded-xl p-4">
              <div className="flex items-start justify-between gap-4">
                <div>
                  <p className="text-sm font-semibold text-slate-900">Trajectory Summary</p>
                  <p className="text-xs text-slate-400 mt-1">{activeSummary?.agent_id || 'unknown-agent'}</p>
                </div>
                <span className={labelClass(activeSummary?.overall_label)}>{activeSummary?.overall_label ?? 'unreviewed'}</span>
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3 mt-4">
                <div>
                  <p className="text-[10px] uppercase tracking-[0.12em] text-slate-400 font-bold">Steps</p>
                  <p className="text-sm font-semibold text-slate-800">{timelineSteps.length}</p>
                </div>
                <div>
                  <p className="text-[10px] uppercase tracking-[0.12em] text-slate-400 font-bold">Events</p>
                  <p className="text-sm font-semibold text-slate-800">{activeSummary?.event_count ?? events.length}</p>
                </div>
                <div>
                  <p className="text-[10px] uppercase tracking-[0.12em] text-slate-400 font-bold">Tokens</p>
                  <p className="text-sm font-semibold text-slate-800">{activeSummary?.total_tokens?.toLocaleString?.() ?? 0}</p>
                </div>
                <div>
                  <p className="text-[10px] uppercase tracking-[0.12em] text-slate-400 font-bold">Cost</p>
                  <p className="text-sm font-semibold text-slate-800">${Number(activeSummary?.total_cost_usd ?? 0).toFixed(5)}</p>
                </div>
                <div>
                  <p className="text-[10px] uppercase tracking-[0.12em] text-slate-400 font-bold">Model</p>
                  <p className="text-sm font-semibold text-slate-800 truncate">{activeSummary?.model || 'unknown'}</p>
                </div>
              </div>
              <div className="mt-3 flex flex-wrap gap-2">
                {(activeSummary ? screeningBadges(activeSummary) : []).map(badge => (
                  <span key={badge} className="badge badge-neutral">{badge}</span>
                ))}
              </div>
            </div>

            <div className="bg-white border border-slate-200 rounded-xl p-4">
              <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3 mb-4">
                <div className="min-w-0">
                  <p className="text-sm font-semibold text-slate-900">Human Label</p>
                  <p className="text-xs text-slate-400 metric-font break-all leading-5">{activeSession || 'Select a trajectory'}</p>
                </div>
                <div className="flex items-center gap-2 self-start sm:self-auto">
                  <button
                    onClick={assist}
                    disabled={!activeSession || assistReview.isPending}
                    className="w-10 h-10 rounded-xl bg-primary/10 border border-primary/25 text-primary flex items-center justify-center transition-all hover:bg-primary/15 hover:border-primary/40 disabled:opacity-50 disabled:cursor-not-allowed"
                    title="Suggest review"
                    aria-label="Suggest review"
                  >
                    <Sparkles className={`w-5 h-5 ${assistReview.isPending ? 'animate-pulse' : ''}`} />
                  </button>
                  <button onClick={save} disabled={!activeSession || saveReview.isPending} className="btn-primary flex items-center gap-1.5 text-xs whitespace-nowrap">
                    <Save className="w-3.5 h-3.5" />
                    Save Review
                  </button>
                </div>
              </div>

              {(assistSuggestion || assistReview.error) && (
                <div className="mb-3 rounded-md border border-indigo-100 bg-indigo-50/60 px-3 py-2">
                  {assistSuggestion ? (
                    <>
                      <div className="flex items-center gap-2">
                        <Sparkles className="w-3.5 h-3.5 text-primary" />
                        <p className="text-xs font-semibold text-slate-800">
                          {assistSuggestion.suggested_label.replace('_', ' ')} · {(assistSuggestion.confidence * 100).toFixed(0)}%
                        </p>
                        {assistSuggestion.failure_type && <span className="badge badge-info">{assistSuggestion.failure_type.replace('_', ' ')}</span>}
                      </div>
                      <p className="mt-1 text-xs text-slate-600 leading-5">{assistSuggestion.critique}</p>
                      {assistSuggestion.failure_event_id && (
                        <p className="mt-1 text-[11px] text-slate-500">
                          Suggested failure step: <span className="font-semibold text-slate-700">{stepLabel(assistantStep)}</span>
                        </p>
                      )}
                      {assistSuggestion.quality_signals.length > 0 && (
                        <div className="mt-2 flex flex-wrap gap-1.5">
                          {assistSuggestion.quality_signals.map(signal => (
                            <span key={signal} className="badge badge-neutral">{signal.replace(/_/g, ' ')}</span>
                          ))}
                        </div>
                      )}
                    </>
                  ) : (
                    <p className="text-xs text-red-600">
                      AI help unavailable. Set <span className="metric-font">AGENTLAND_REVIEW_ASSIST_OPENAI_API_KEY</span> on the proxy.
                    </p>
                  )}
                </div>
              )}

              <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-3">
                <label className="text-xs text-slate-500">
                  Overall label
                  <select className="input-field mt-1 w-full" value={overallLabel} onChange={e => setOverallLabel(e.target.value as SaveTrajectoryReviewRequest['overall_label'])}>
                    {labelOptions.map(v => <option key={v} value={v}>{v.replace('_', ' ')}</option>)}
                  </select>
                </label>
                <label className="text-xs text-slate-500">
                  Failure type
                  <select className="input-field mt-1 w-full" value={failureType} onChange={e => setFailureType(e.target.value)} disabled={overallLabel === 'good'}>
                    {failureTypes.map(v => <option key={v} value={v}>{v.split('_').join(' ')}</option>)}
                  </select>
                </label>
                <label className="text-xs text-slate-500 col-span-1 sm:col-span-2">
                  Failure step
                  <select className="input-field mt-1 w-full" value={failureEventId} onChange={e => setFailureEventId(e.target.value)} disabled={overallLabel === 'good'}>
                    <option value="">No specific step</option>
                    {timelineSteps.map(step => {
                      const label = stepLabel(step)
                      return <option key={`${step.kind}-${step.index}-${step.eventId}`} value={step.eventId}>{label}</option>
                    })}
                  </select>
                </label>
                <label className="text-xs text-slate-500 col-span-1 sm:col-span-2 xl:col-span-4">
                  Reviewer notes
                  <textarea className="input-field mt-1 w-full min-h-[76px]" value={notes} onChange={e => setNotes(e.target.value)} placeholder="Why is this trajectory good, bad, or worth reviewing?" />
                </label>
              </div>
              {selectedStep && (
                <p className="text-xs text-slate-500 mt-3">Selected failure step: <span className="metric-font">{selectedStep.id}</span></p>
              )}
            </div>

            <div className="bg-white border border-slate-200 rounded-xl overflow-hidden">
              <div className="px-4 py-3 border-b border-slate-100 flex items-center justify-between">
                <div>
                  <p className="text-sm font-semibold text-slate-900">Trajectory Timeline</p>
                  <p className="text-xs text-slate-400">{timelineSteps.length} agent steps · {events.length} raw events</p>
                </div>
                <div className="inline-flex rounded-lg border border-slate-200 bg-slate-50 p-0.5">
                  <button
                    onClick={() => setTimelineMode('readable')}
                    className={`px-3 py-1 text-xs rounded-md transition-colors ${timelineMode === 'readable' ? 'bg-white text-primary shadow-sm' : 'text-slate-500 hover:text-slate-700'}`}
                  >
                    Readable
                  </button>
                  <button
                    onClick={() => setTimelineMode('raw')}
                    className={`px-3 py-1 text-xs rounded-md transition-colors ${timelineMode === 'raw' ? 'bg-white text-primary shadow-sm' : 'text-slate-500 hover:text-slate-700'}`}
                  >
                    Raw JSON
                  </button>
                </div>
              </div>
              <div className="divide-y divide-slate-100">
                {timelineSteps.map(step => {
                  const turn = { request: step.request, response: step.response, index: step.index }
                  const content = step.kind === 'model' ? turnContent(turn) : null
                  const toolContent = step.kind === 'tool' ? toolStepContent(step) : null
                  const eventId = step.eventId
                  const marked = eventId === failureEventId
                  return (
                    <div
                      key={`${step.kind}-${step.index}-${eventId}`}
                      ref={node => { if (eventId) timelineRefs.current[eventId] = node }}
                      className={`px-4 py-3 transition-colors ${
                        marked
                          ? 'bg-amber-100 border-l-4 border-amber-500 shadow-[inset_0_0_0_1px_rgba(245,158,11,0.28)]'
                          : 'border-l-4 border-transparent'
                      }`}
                    >
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <div className="flex items-center gap-2">
                            <span className={`w-5 h-5 rounded-full flex items-center justify-center text-[11px] font-bold ${step.kind === 'tool' ? 'bg-emerald-100 text-emerald-700' : 'bg-primary/10 text-primary'}`}>{step.index}</span>
                            <span className="text-sm font-semibold text-slate-800">{step.kind === 'tool' ? 'Tool execution' : 'Model turn'}</span>
                            <span className="badge badge-neutral">{step.kind === 'tool' ? toolContent?.name : turn.request?.provider || turn.response?.provider || 'unknown'}</span>
                            {turn.response?.status_code != null && <span className="badge badge-info">HTTP {turn.response.status_code}</span>}
                            {turn.response?.finish_reason === 'tool_calls' && step.kind === 'model' && <span className="badge badge-info">requested tool</span>}
                            {marked && <span className="badge badge-warning">failure step</span>}
                          </div>
                          <p className="text-xs text-slate-400 mt-1">{fmtTime(turn.request?.timestamp || turn.response?.timestamp)} · {turn.request?.model || turn.response?.model || 'local tool'} · {turn.response?.latency_ms != null ? `${turn.response.latency_ms}ms` : step.kind === 'tool' ? 'local execution' : 'latency n/a'}</p>
                        </div>
                        <div className="flex items-center gap-2">
                          <p className="text-xs metric-font text-slate-500">{turn.response?.total_tokens ?? 0} tokens</p>
                          <button
                            className="btn-secondary text-xs py-1 px-2"
                            disabled={overallLabel === 'good' || !eventId}
                            onClick={() => setFailureEventId(eventId)}
                          >
                            Mark failure here
                          </button>
                        </div>
                      </div>

                      {timelineMode === 'readable' ? (
                        <div className="mt-2 space-y-2">
                          {step.kind === 'tool' ? (
                            <>
                              <div className="rounded-md border border-emerald-100 bg-emerald-50/50 px-3 py-2">
                                <p className="text-[10px] uppercase tracking-[0.1em] text-emerald-700 font-bold">Tool Call</p>
                                <p className="mt-1 text-sm text-slate-800">{toolContent?.args}</p>
                              </div>
                              <div className="rounded-md border border-emerald-100 bg-white px-3 py-2">
                                <p className="text-[10px] uppercase tracking-[0.1em] text-emerald-700 font-bold">Tool Result</p>
                                <pre className="mt-1 max-h-44 overflow-auto text-xs text-slate-800 whitespace-pre-wrap leading-5">{toolContent?.result}</pre>
                              </div>
                            </>
                          ) : (
                            <>
                              <div className="rounded-md border border-slate-100 bg-slate-50 px-3 py-2">
                                <p className="text-[10px] uppercase tracking-[0.1em] text-slate-400 font-bold">Prompt</p>
                                <p className="mt-1 text-sm text-slate-800 whitespace-pre-wrap leading-5">{content?.prompt}</p>
                              </div>
                              <div className="rounded-md border border-slate-100 bg-white px-3 py-2">
                                <p className="text-[10px] uppercase tracking-[0.1em] text-slate-400 font-bold">Response</p>
                                <p className="mt-1 text-sm text-slate-800 whitespace-pre-wrap leading-5">{content?.answer}</p>
                              </div>
                            </>
                          )}
                          <div className="flex flex-wrap gap-1.5">
                            {step.kind === 'model' && content?.meta.map(item => (
                              <span key={item} className="badge badge-neutral">{item}</span>
                            ))}
                            {step.kind === 'tool' && <span className="badge badge-success">captured from transcript</span>}
                          </div>
                        </div>
                      ) : (
                        <div className="mt-2 grid grid-cols-1 lg:grid-cols-2 gap-2">
                          {step.kind === 'tool' ? (
                            <>
                              <pre className="max-h-56 overflow-auto rounded-md bg-slate-950 text-slate-100 text-[11px] p-2 whitespace-pre-wrap">{JSON.stringify(step.toolCall ?? {}, null, 2)}</pre>
                              <pre className="max-h-56 overflow-auto rounded-md bg-slate-950 text-slate-100 text-[11px] p-2 whitespace-pre-wrap">{JSON.stringify(step.toolResult ?? {}, null, 2)}</pre>
                            </>
                          ) : (
                            <>
                              <pre className="max-h-56 overflow-auto rounded-md bg-slate-950 text-slate-100 text-[11px] p-2 whitespace-pre-wrap">{turn.request ? payloadText(turn.request) : 'No request event'}</pre>
                              <pre className="max-h-56 overflow-auto rounded-md bg-slate-950 text-slate-100 text-[11px] p-2 whitespace-pre-wrap">{turn.response ? payloadText(turn.response) : 'No response event'}</pre>
                            </>
                          )}
                        </div>
                      )}
                    </div>
                  )
                })}
                {events.length === 0 && (
                  <div className="py-16 text-center text-slate-400">
                    <ClipboardCheck className="w-10 h-10 mx-auto mb-3 text-slate-300" />
                    <p className="text-sm font-medium">Select a trajectory to review</p>
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
