import type { LinkComponentProps } from "@cloudflare/kumo";
import { forwardRef } from "react";
import { Link } from "react-router";

/** 交给 Kumo 的 LinkProvider：Kumo 渲染的链接走 React Router 的客户端导航，不整页刷新。 */
export const AppLink = forwardRef<HTMLAnchorElement, LinkComponentProps>(
  ({ href, to, ...rest }, ref) => <Link ref={ref} to={href ?? to ?? ""} {...rest} />,
);
AppLink.displayName = "AppLink";
