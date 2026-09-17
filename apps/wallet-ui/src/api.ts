export type Status='pending'|'settled'|'failed'|'uncertain';
export interface Asset {asset_id:string;name:string;ticker:string;precision:number}
export interface Holding {asset:Asset;outbound:string;onchain:string}
export interface Wallet {wallet_name?:string;holdings:Holding[];sats:null;network:string}
export interface Plan {recipient?:{identifier:string;authoritative_domain:string};plan_id:string;request:{asset_id:string;amount:string;invoice:string;payment_hash:string;carrier_msat:string;expires_at:number};available_balance:string;policy:{decision:string;reason?:string}}
export interface MachinePlan {plan_id:string;url:string;cost_sats:number;available_sats:number;auto_approve_up_to_sats:number;policy:{decision:string;reason?:string}}
export interface Purchase {resource_status:string;url:string;cost_sats:number;auto_approved:boolean;auto_approve_up_to_sats:number;policy:{decision:string;reason?:string};payment_hash:string|null;payment_status:Status|null;http_status:number|null;resource:Record<string,unknown>|null;plan:MachinePlan|null}
export interface Activity {kind?:"rgb_transfer"|"machine_purchase";resource?:string;auto_approved?:boolean;payment_hash:string;asset_id:string;amount:string;timestamp:number;direction:string;status:Status}
export interface Session {machine_pending?:MachinePlan|null;machine_result?:Purchase|null;csrf:string;busy:boolean;pending:Plan|null;events:{id:number;kind:string;text:string}[]}
export class ApiError extends Error {constructor(public status:number,message:string){super(message)}}
export async function api<T>(path:string,csrf?:string,body?:object):Promise<T> {
  const response=await fetch(`/api${path}`,{method:body===undefined?'GET':'POST',headers:body===undefined?{}:{'Content-Type':'application/json','X-Wallet-CSRF':csrf??''},body:body===undefined?undefined:JSON.stringify(body),cache:'no-store'});
  if(!response.ok){let message='Wallet backend unavailable. Check the local connection.';try{message=(await response.json()).error??message}catch{/* no raw server bodies */}throw new ApiError(response.status,message)}
  return response.status===202?undefined as T:response.json();
}
export function approvePlan(planId:string,csrf:string,approve:boolean){return api(`/approvals/${encodeURIComponent(planId)}/${approve?'approve':'reject'}`,csrf,{})}
export function units(raw:string,precision=0):string {
  try {const value=BigInt(raw);const scale=10n**BigInt(precision);const whole=(value/scale).toLocaleString('en-US');const fraction=(value%scale).toString().padStart(precision,'0').replace(/0+$/,'');return whole+(fraction?`.${fraction}`:'')}catch{return 'Unavailable'}
}
export function short(text:string){return text.length>22?`${text.slice(0,10)}…${text.slice(-8)}`:text}
