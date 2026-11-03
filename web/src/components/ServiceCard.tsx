/**
 * A service card: icon, name, description, status and live widget stats.
 */
import type { ReactNode } from "react";
import type { Service, ServiceStatus } from "../api/types";
import { Icon } from "./Icon";
import { StatusBadge } from "./StatusBadge";
import { ServiceWidget } from "./widgets/ServiceWidget";

/** Props of {@link ServiceCard}. */
export interface ServiceCardProps {
  service: Service;
  status?: ServiceStatus;
  /** When true the card is not a link (edit mode). */
  editing?: boolean;
  /** Edit-mode controls rendered in the corner. */
  actions?: ReactNode;
}

/** Renders a service card; a link to `href` outside edit mode. */
export function ServiceCard({ service, status, editing = false, actions }: ServiceCardProps) {
  const body = (
    <>
      <div className="card-main">
        <Icon icon={service.icon} kind={service.iconKind} name={service.name} />
        <div className="card-text">
          <div className="card-name">{service.name}</div>
          {service.description && <div className="card-desc">{service.description}</div>}
        </div>
        <StatusBadge check={service.check} status={status} />
      </div>
      <ServiceWidget service={service} />
      {actions}
    </>
  );
  if (service.href && !editing) {
    return (
      <a
        className="card"
        href={service.href}
        target="_blank"
        rel="noopener noreferrer"
        data-testid="card"
      >
        {body}
      </a>
    );
  }
  return (
    <div className="card" data-testid="card">
      {body}
    </div>
  );
}
