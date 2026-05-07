export interface Adxl345Sample {
  x: number;
  y: number;
  z: number;
}

export interface Adxl345VisualizerProps {
  sample: Adxl345Sample;
  history: Adxl345Sample[];
  onSampleChange: (sample: Adxl345Sample) => void;
}

function clampSample(value: number): number {
  return Math.max(-512, Math.min(512, Math.round(value)));
}

export function Adxl345Visualizer({ sample, history, onSampleChange }: Adxl345VisualizerProps) {
  const points = history.slice(-40);
  const width = 280;
  const height = 96;
  const line = (axis: keyof Adxl345Sample) =>
    points
      .map((point, index) => {
        const x = points.length <= 1 ? 0 : (index / (points.length - 1)) * width;
        const y = height / 2 - (point[axis] / 512) * (height / 2 - 8);
        return `${x.toFixed(1)},${y.toFixed(1)}`;
      })
      .join(' ');

  return (
    <div className="adxl345-visualizer">
      <div className="adxl345-board" aria-label="ADXL345 breakout board">
        <div className="adxl345-chip">ADXL345</div>
        <div className="adxl345-pins">VCC GND SDA SCL</div>
      </div>
      <div className="adxl345-controls">
        {(['x', 'y', 'z'] as const).map((axis) => (
          <label key={axis} className="axis-control">
            <span>{axis.toUpperCase()}</span>
            <input
              type="range"
              min="-512"
              max="512"
              value={sample[axis]}
              onChange={(event) =>
                onSampleChange({ ...sample, [axis]: clampSample(Number(event.target.value)) })
              }
            />
            <output>{sample[axis]}</output>
          </label>
        ))}
      </div>
      <svg
        className="adxl345-chart"
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label="Acceleration chart"
      >
        <polyline points={line('x')} fill="none" stroke="#e83e8c" strokeWidth="2" />
        <polyline points={line('y')} fill="none" stroke="#27c93f" strokeWidth="2" />
        <polyline points={line('z')} fill="none" stroke="#569cd6" strokeWidth="2" />
      </svg>
    </div>
  );
}
