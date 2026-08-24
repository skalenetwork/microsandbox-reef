import type { APIRoute } from "astro";
import { files, route } from "../content";

export const getStaticPaths = () =>
  Object.entries(files).map(([file, body]) => ({ params: { path: route(file) }, props: { body } }));

export const GET: APIRoute<{ body: string }> = ({ props }) => new Response(props.body);
