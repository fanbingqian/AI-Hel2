import { useMemo, useRef } from "react";
import { useFrame } from "@react-three/fiber";
import * as THREE from "three";

const SPHERE_R = 2.5;

export function SphereShell() {
  const shellRef = useRef<THREE.Mesh>(null);

  const wireGeo = useMemo(() => new THREE.IcosahedronGeometry(SPHERE_R, 5), []);

  return (
    <>
      {/* Wireframe — dark subtle lines */}
      <mesh geometry={wireGeo}>
        <meshBasicMaterial
          color="#1a2a1a"
          wireframe
          transparent
          opacity={0.10}
          depthWrite={false}
        />
      </mesh>
      {/* Shell shader — fresnel edge glow */}
      <mesh ref={shellRef} geometry={new THREE.SphereGeometry(SPHERE_R - 0.08, 64, 64)}>
        <shaderMaterial
          uniforms={{ uTime: { value: 0 } }}
          vertexShader={/*glsl*/`
            varying vec3 vNormal; varying vec3 vPos;
            void main() {
              vec4 wp = modelMatrix * vec4(position, 1.0);
              vPos = wp.xyz;
              vNormal = normalize(mat3(modelMatrix) * normal);
              gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
            }
          `}
          fragmentShader={/*glsl*/`
            varying vec3 vNormal; varying vec3 vPos;
            uniform float uTime;
            void main() {
              vec3 V = normalize(cameraPosition - vPos);
              float f = 1.0 - abs(dot(V, vNormal));
              f = pow(f, 4.0);
              float a = 0.02 + f * 0.05;
              gl_FragColor = vec4(0.03, 0.06, 0.03, a);
            }
          `}
          transparent
          depthWrite={false}
        />
      </mesh>
    </>
  );
}

export { SPHERE_R };
