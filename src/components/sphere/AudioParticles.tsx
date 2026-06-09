import { useRef, useMemo } from "react";
import { useFrame } from "@react-three/fiber";
import { useVoiceStore } from "../../stores/voiceStore";
import * as THREE from "three";
import type { SphereNode } from "../../hooks/useSphereNodes";

interface Props {
  nodes: SphereNode[];
}

const PARTICLE_COUNT = 2000;

export function AudioParticles({ nodes }: Props) {
  const pointsRef = useRef<THREE.Points>(null);
  const volume = useVoiceStore((s) => s.volume);
  const spectrum = useVoiceStore((s) => s.spectrum);

  const { positions, basePositions, colors } = useMemo(() => {
    const pos = new Float32Array(PARTICLE_COUNT * 3);
    const base = new Float32Array(PARTICLE_COUNT * 3);
    const col = new Float32Array(PARTICLE_COUNT * 3);

    for (let i = 0; i < PARTICLE_COUNT; i++) {
      const nodeIdx = Math.floor(Math.random() * Math.max(nodes.length, 1));
      const node = nodes[nodeIdx] || { position: [0, 0, 0] as [number, number, number], radius: 0.05 };

      const phi = Math.random() * Math.PI * 2;
      const theta = Math.random() * Math.PI;
      const r = node.radius + Math.random() * 0.3;

      pos[i * 3] = node.position[0] + r * Math.sin(theta) * Math.cos(phi);
      pos[i * 3 + 1] = node.position[1] + r * Math.cos(theta);
      pos[i * 3 + 2] = node.position[2] + r * Math.sin(theta) * Math.sin(phi);

      base[i * 3] = pos[i * 3];
      base[i * 3 + 1] = pos[i * 3 + 1];
      base[i * 3 + 2] = pos[i * 3 + 2];

      const t = Math.random();
      col[i * 3] = 0.15 + t * 0.85;
      col[i * 3 + 1] = 0.78 + t * 0.22;
      col[i * 3 + 2] = 0.48 + t * 0.52;
    }

    return { positions: pos, basePositions: base, colors: col };
  }, [nodes]);

  useFrame(() => {
    if (!pointsRef.current) return;
    const posArr = pointsRef.current.geometry.attributes.position.array as Float32Array;

    for (let i = 0; i < PARTICLE_COUNT; i++) {
      const freqIdx = Math.floor((i / PARTICLE_COUNT) * spectrum.length);
      const freqVal = spectrum[freqIdx] / 255;

      const bx = basePositions[i * 3];
      const by = basePositions[i * 3 + 1];
      const bz = basePositions[i * 3 + 2];
      const len = Math.sqrt(bx * bx + by * by + bz * bz) || 1;
      const nx = bx / len;
      const ny = by / len;
      const nz = bz / len;

      const displace = freqVal * 0.5 * (1 + volume);
      const springTarget = displace * 0.8;

      posArr[i * 3] = bx + nx * (springTarget + (Math.random() - 0.5) * 0.02);
      posArr[i * 3 + 1] = by + ny * (springTarget + (Math.random() - 0.5) * 0.02);
      posArr[i * 3 + 2] = bz + nz * (springTarget + (Math.random() - 0.5) * 0.02);
    }

    pointsRef.current.geometry.attributes.position.needsUpdate = true;
  });

  return (
    <points ref={pointsRef}>
      <bufferGeometry>
        <bufferAttribute
          attach="attributes-position"
          count={PARTICLE_COUNT}
          array={positions}
          itemSize={3}
        />
        <bufferAttribute
          attach="attributes-color"
          count={PARTICLE_COUNT}
          array={colors}
          itemSize={3}
        />
      </bufferGeometry>
      <pointsMaterial
        size={0.02}
        vertexColors
        transparent
        depthWrite={false}
        blending={THREE.AdditiveBlending}
      />
    </points>
  );
}
