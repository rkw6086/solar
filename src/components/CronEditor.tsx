import { ConfigProvider, theme } from "antd";
import { Cron } from "react-js-cron";
import "react-js-cron/styles.css";

export function CronEditor({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  return <div className="solar-cron-editor">
    <ConfigProvider theme={{ algorithm: theme.darkAlgorithm, token: { colorPrimary: "#e7ad66", colorBgContainer: "#24252b", borderRadius: 8, fontSize: 13 } }}>
      <Cron value={value} setValue={onChange} humanizeLabels clockFormat="12-hour-clock" clearButton={false} />
    </ConfigProvider>
    <label className="field"><span>Cron expression</span><input aria-label="Cron expression" value={value} onChange={event => onChange(event.target.value)} required placeholder="0 9 * * 1-5" />
    <small>Schedules follow this computer’s local time. Example: weekdays at 9 AM is <code>0 9 * * 1-5</code>.</small></label>
  </div>;
}
