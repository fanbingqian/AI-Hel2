import { useMemo } from "react";
import { Line } from "@react-three/drei";
import * as THREE from "three";
import type { Relation } from "../../types/knowledge";
import type { SphereNode } from "../../hooks/useSphereNodes";

interface Props {
  nodes: SphereNode[];
  relations: Relation[];
  edgeOpacity?: number;
}

const SPHERE_R = 2.5;

function bezierPoints(
  p1: [number, number, number],
  p2: [number, number, number],
  segments: number = 32,
): [number, number, number][] {
  const a = new THREE.Vector3(...p1);
  const b = new THREE.Vector3(...p2);
  const mid = new THREE.Vector3().addVectors(a, b).multiplyScalar(0.5);
  const len = mid.length();
  if (len > 0.001) {
    mid.multiplyScalar((SPHERE_R + 1.2) / len);
  }
  const curve = new THREE.QuadraticBezierCurve3(a, mid, b);
  return curve.getPoints(segments).map((v) => [v.x, v.y, v.z] as [number, number, number]);
}

export function EdgeLines({ nodes, relations, edgeOpacity = 0.9 }: Props) {
  const nodeMap = useMemo(
    () => new Map(nodes.map((n) => [n.id, n])),
    [nodes],
  );

  const lines = useMemo(() => {
    return relations
      .filter((r) => nodeMap.has(r.from_id) && nodeMap.has(r.to_id))
      .map((r) => {
        const src = nodeMap.get(r.from_id)!;
        const tgt = nodeMap.get(r.to_id)!;
        return {
          key: r.id,
          points: bezierPoints(src.position, tgt.position),
          weight: r.weight || 0.5,
        };
      });
  }, [relations, nodeMap]);

  return (
    <group>
      {lines.map((line) => {
        const alpha = edgeOpacity * (0.3 + line.weight * 0.7);
        return (
          <Line
            key={line.key}
            points={line.points}
            color="#ffffff"
            lineWidth={0.4}
            transparent
            depthWrite={false}
            blending={THREE.AdditiveBlending}
            opacity={alpha}
          />
        );
      })}
    </group>
  );
}
