import { useMemo } from "react";
import type { Entity, Relation } from "../types/knowledge";

export interface SphereNode {
  id: string;
  name: string;
  entityType: string;
  confidence: number;
  position: [number, number, number];
  radius: number;
  degree: number;
}

const SPHERE_R = 2.5;

function computeDegrees(entities: Entity[], relations: Relation[]): Map<string, number> {
  const deg = new Map<string, number>();
  for (const e of entities) deg.set(e.id, 0);
  for (const r of relations) {
    deg.set(r.from_id, (deg.get(r.from_id) ?? 0) + 1);
    deg.set(r.to_id, (deg.get(r.to_id) ?? 0) + 1);
  }
  return deg;
}

function fibonacciSphere(n: number, total: number): [number, number, number] {
  const phi = Math.acos(1 - 2 * (n + 0.5) / total);
  const theta = Math.PI * (1 + Math.sqrt(5)) * n;
  return [
    SPHERE_R * Math.sin(phi) * Math.cos(theta),
    SPHERE_R * Math.cos(phi),
    SPHERE_R * Math.sin(phi) * Math.sin(theta),
  ];
}

export function useSphereNodes(entities: Entity[], relations: Relation[]): SphereNode[] {
  return useMemo(() => {
    if (entities.length === 0) return [];

    const degrees = computeDegrees(entities, relations);
    const maxDeg = Math.max(1, ...Array.from(degrees.values()));

    const total = entities.length;
    const nodes: SphereNode[] = entities.map((entity, i) => {
      const degree = degrees.get(entity.id) ?? 0;
      const minR = 0.03;
      const maxR = 0.08;
      const radius = minR + (maxR - minR) * (Math.log(degree + 1) / Math.log(maxDeg + 1));

      // Add slight random jitter off the sphere surface
      const base = fibonacciSphere(i, total);
      const jitter = 0.15;
      const len = Math.sqrt(base[0] ** 2 + base[1] ** 2 + base[2] ** 2) || 1;
      const nx = base[0] / len;
      const ny = base[1] / len;
      const nz = base[2] / len;

      return {
        id: entity.id,
        name: entity.name,
        entityType: entity.entity_type,
        confidence: entity.confidence,
        position: [
          base[0] + nx * (Math.random() - 0.5) * jitter,
          base[1] + ny * (Math.random() - 0.5) * jitter,
          base[2] + nz * (Math.random() - 0.5) * jitter,
        ],
        radius,
        degree,
      };
    });

    return nodes;
  }, [entities, relations]);
}
