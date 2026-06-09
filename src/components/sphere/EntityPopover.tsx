import type { EntityDetail } from "../../types/knowledge";
import { getTypeColor } from "../../types/knowledge";
import styles from "./EntityPopover.module.css";

interface Props {
  detail: EntityDetail;
  entityName: string;
  entityType: string;
  onReference: () => void;
  onClose: () => void;
}

export function EntityPopover({ detail, entityName, entityType, onReference, onClose }: Props) {
  const typeColor = getTypeColor(entityType);
  const allRelations = [...detail.inbound_relations, ...detail.outbound_relations].slice(0, 3);

  return (
    <div className={styles.popover}>
      <div className={styles.header}>
        <span className={styles.typeDot} style={{ backgroundColor: typeColor }} />
        <span className={styles.name}>{entityName}</span>
        <button className={styles.close} onClick={onClose}>x</button>
      </div>
      <div className={styles.body}>
        <div className={styles.meta}>
          <span className={styles.metaItem}>{entityType}</span>
          <span className={styles.metaItem}>置信度 {(detail.entity.confidence * 100).toFixed(0)}%</span>
        </div>
        {detail.entity.description && (
          <p className={styles.desc}>{detail.entity.description.slice(0, 120)}</p>
        )}
        {detail.entity.properties && Object.keys(detail.entity.properties).length > 0 && (
          <div className={styles.props}>
            {Object.entries(detail.entity.properties).slice(0, 5).map(([key, val]: [string, any]) => (
              <span key={key} className={styles.propTag} title={JSON.stringify(val)}>
                {key}: {typeof val?.value === "object" ? JSON.stringify(val.value) : String(val?.value ?? val)}
              </span>
            ))}
          </div>
        )}
        {allRelations.length > 0 && (
          <div className={styles.relations}>
            {allRelations.map((r, i) => (
              <div key={i} className={styles.relItem}>
                <span className={styles.relType}>{r.relation_type}</span>
              </div>
            ))}
          </div>
        )}
      </div>
      <button className={styles.refBtn} onClick={onReference}>
        引用到对话
      </button>
    </div>
  );
}
