import { useEffect, useRef, useState } from "react";
import { api } from "../api";

interface Props {
  lines: string[];
  onClear: () => void;
}

export default function Logs({ lines, onClear }: Props) {
  const [filter, setFilter] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);
  const endRef = useRef<HTMLDivElement | null>(null);

  const filtered = filter
    ? lines.filter((line) => line.toLowerCase().includes(filter.toLowerCase()))
    : lines;

  useEffect(() => {
    if (autoScroll) {
      endRef.current?.scrollIntoView({ block: "end" });
    }
  }, [filtered.length, autoScroll]);

  const clear = async () => {
    try {
      await api.clearLogs();
    } catch {
      /* backend buffer clear is best-effort */
    }
    onClear();
  };

  return (
    <div className="page page-logs">
      <header className="page-head">
        <h1>Logs</h1>
        <div className="head-actions">
          <input
            className="search"
            placeholder="filter"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <label className="check">
            <input
              type="checkbox"
              checked={autoScroll}
              onChange={(e) => setAutoScroll(e.target.checked)}
            />
            auto-scroll
          </label>
          <button className="btn" onClick={() => void clear()}>
            Clear
          </button>
        </div>
      </header>

      <section className="card log-card">
        {filtered.length === 0 ? (
          <p className="muted">
            {lines.length === 0
              ? "Nothing yet — gateway and registry activity will stream here."
              : "No lines match the filter."}
          </p>
        ) : (
          <pre className="log-view">
            {filtered.map((line, i) => (
              <div
                key={i}
                className={
                  /\bERROR\b/.test(line)
                    ? "log-line log-err"
                    : /\bWARN\b/.test(line)
                      ? "log-line log-warn"
                      : /\bINFO\b/.test(line)
                        ? "log-line log-info"
                        : "log-line"
                }
              >
                {line}
              </div>
            ))}
            <div ref={endRef} />
          </pre>
        )}
      </section>
    </div>
  );
}
