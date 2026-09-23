import {render,screen,fireEvent,waitFor} from '@testing-library/react';
import {describe,it,expect,vi} from 'vitest';
import App,{BtcApprovalSheet,ReceiveAddress,ApprovalSheet,ActivityList,PaymentReceipt,MachineApprovalSheet,MachineReceipt,PaymentRails} from './App';
import {approvePlan,units,type Plan,type Asset,type Status} from './api';
const asset:Asset={asset_id:'rgb:demo',name:'Demo Dollar',ticker:'R402USD',precision:0};
const plan:Plan={plan_id:'bound-plan-1',request:{asset_id:'rgb:demo',amount:'5',invoice:'lnbcrt-bound-invoice',payment_hash:'hash',carrier_msat:'3000000',expires_at:9999999999},available_balance:'490',policy:{decision:'require_approval',reason:'amount meets threshold'}};
function backend(error?:string){
 return vi.spyOn(globalThis,'fetch').mockImplementation(async(url,options)=>{
  if(options?.method==='POST')return new Response(null,{status:202});
  if(String(url).endsWith('/session'))return new Response(JSON.stringify({csrf:'test-csrf',busy:false,pending:null,events:error?[{id:1,kind:'error',text:error}]:[]}));
  if(String(url).endsWith('/wallet'))return new Response(JSON.stringify({holdings:[{asset,outbound:'490',onchain:'400'}],sats:null,network:'regtest'}));
  return new Response('[]');
 });
}
describe('wallet UI',()=>{
 it('renders actual balance without fabricating sats',async()=>{backend();render(<App/>);expect(await screen.findByTestId('primary-balance')).toBeInTheDocument();await waitFor(()=>expect(screen.getByTestId('primary-balance')).toHaveTextContent('490'));expect(screen.getByText('Spendable outbound Lightning capacity')).toBeInTheDocument()});
 it('submits natural language to the agent API',async()=>{const fetch=backend();render(<App/>);await waitFor(()=>expect(screen.getByTestId('primary-balance')).toHaveTextContent('490'));fireEvent.click(screen.getByRole('button',{name:'Agent'}));fireEvent.change(screen.getByLabelText('Message your wallet'),{target:{value:'Pay this invoice: lnbcrt-demo'}});fireEvent.click(screen.getByRole('button',{name:'Send message'}));await waitFor(()=>expect(fetch).toHaveBeenCalledWith('/api/agent/message',expect.objectContaining({method:'POST',body:JSON.stringify({message:'Pay this invoice: lnbcrt-demo'})}))) });
 it('shows authoritative confirmation details in a dedicated dialog',()=>{render(<ApprovalSheet plan={plan} asset={asset} busy={false} onAction={()=>{}}/>);expect(screen.getByRole('dialog')).toHaveAccessibleName('Confirm payment');expect(screen.getByText('Manual approval required')).toBeInTheDocument();expect(screen.getByText('3,000 sats')).toBeInTheDocument();expect(screen.getByText('lnbcrt-bound-invoice')).toBeInTheDocument()});
 it('approval sends only plan identity and csrf, never editable payment fields',async()=>{const fetch=vi.spyOn(globalThis,'fetch').mockResolvedValue(new Response(null,{status:202}));await approvePlan('bound-plan-1','test-csrf',true);expect(fetch).toHaveBeenCalledWith('/api/approvals/bound-plan-1/approve',expect.objectContaining({method:'POST',body:'{}',headers:{'Content-Type':'application/json','X-Wallet-CSRF':'test-csrf'}}))});
 it('reject invokes only the reject action',async()=>{const action=vi.fn();render(<ApprovalSheet plan={plan} asset={asset} busy={false} onAction={action}/>);fireEvent.click(screen.getByRole('button',{name:'Cancel'}));expect(action).toHaveBeenCalledWith(false);const fetch=vi.spyOn(globalThis,'fetch').mockResolvedValue(new Response(null,{status:202}));await approvePlan(plan.plan_id,'test-csrf',false);expect(fetch).toHaveBeenCalledWith('/api/approvals/bound-plan-1/reject',expect.objectContaining({body:'{}'}))});
 for(const status of ['pending','settled','failed','uncertain'] as Status[])it(`renders ${status} only from authoritative activity`,()=>{render(<ActivityList assets={[asset]} items={[{payment_hash:'hash',asset_id:'rgb:demo',amount:'5',timestamp:1789100000,direction:'sent',status}]}/>);expect(screen.getByText(status[0].toUpperCase()+status.slice(1))).toBeInTheDocument()});
 it('displays an invalid invoice error without replacing it with success',async()=>{backend('Wallet validation rejected the payment: invalid invoice');render(<App/>);fireEvent.click(screen.getByRole('button',{name:'Agent'}));expect(await screen.findByText('Wallet validation rejected the payment: invalid invoice')).toBeInTheDocument();expect(screen.queryByText('Settled')).not.toBeInTheDocument()});
 it('keeps large base-unit amounts exact',()=>{expect(units('18446744073709551615',2)).toBe('184,467,440,737,095,516.15')});
 it('disables approval while another action is in flight',()=>{const action=vi.fn();render(<ApprovalSheet plan={plan} asset={asset} busy={true} onAction={action}/>);fireEvent.click(screen.getByRole('button',{name:'Approve payment'}));expect(action).not.toHaveBeenCalled()});
});

it('receipt updates from pending to settled authoritative activity',()=>{const item={payment_hash:'hash',asset_id:'rgb:demo',amount:'5',timestamp:1789100000,direction:'sent',status:'pending' as Status};const {rerender}=render(<PaymentReceipt item={item} asset={asset}/>);expect(screen.getByText('Pending')).toBeInTheDocument();rerender(<PaymentReceipt item={{...item,status:'settled'}} asset={asset}/>);expect(screen.getByText('Settled')).toBeInTheDocument();expect(screen.queryByText('Pending')).not.toBeInTheDocument()});

it('machine service requires explicit approval above the returned threshold',()=>{const action=vi.fn();render(<MachineApprovalSheet plan={{plan_id:'machine-bound',url:'http://127.0.0.1:3040/premium/extended',cost_sats:50,available_sats:1000,auto_approve_up_to_sats:10,policy:{decision:'require_approval'}}} busy={false} onAction={action}/>);expect(screen.getByRole('dialog')).toHaveAccessibleName('Approve service purchase');expect(action).not.toHaveBeenCalled();fireEvent.click(screen.getByRole('button',{name:'Cancel'}));expect(action).toHaveBeenCalledWith(false)});
it('machine receipt distinguishes paid from unlocked and never exposes credentials',()=>{render(<MachineReceipt purchase={{resource_status:'paid_resource_unavailable',url:'http://127.0.0.1:3040/premium/report',cost_sats:3,auto_approved:true,auto_approve_up_to_sats:10,policy:{decision:'allow_auto'},payment_hash:'hash',payment_status:'settled',http_status:null,resource:null,plan:null}}/>);expect(screen.getByText('Settled')).toBeInTheDocument();expect(screen.queryByText('Unlocked')).not.toBeInTheDocument();expect(screen.getByText('Auto-approved (up to 10 sats)')).toBeInTheDocument()});
it('machine activity uses sats and a separate service label',()=>{render(<ActivityList assets={[asset]} items={[{kind:'machine_purchase',payment_hash:'hash',asset_id:'BTC',amount:'3',timestamp:1789100000,direction:'sent',status:'settled',auto_approved:true,resource:'http://127.0.0.1:3040/premium/report'}]}/>);expect(screen.getByText('Machine service')).toBeInTheDocument();expect(screen.getByText('−3 sats')).toBeInTheDocument();expect(screen.queryByText('Sent R402USD')).not.toBeInTheDocument()});

it('recipient approval identifies the immutable recipient and domain',()=>{render(<ApprovalSheet plan={{...plan,recipient:{identifier:'alice@example.com',authoritative_domain:'example.com'},request:{...plan.request,invoice:''}}} asset={asset} busy={false} onAction={()=>{}}/>);expect(screen.getByText('alice@example.com')).toBeInTheDocument();expect(screen.getByText('example.com')).toBeInTheDocument();expect(screen.getByText('Manual approval required')).toBeInTheDocument();expect(screen.getByText('Amount: 5 base units')).toBeInTheDocument()});

it('renders received activity without inventing sender identity',()=>{render(<ActivityList assets={[asset]} items={[{payment_hash:'incoming',asset_id:'rgb:demo',amount:'5',timestamp:1789100000,direction:'received',status:'settled'}]}/>);expect(screen.getByText('Received R402USD')).toBeInTheDocument();expect(screen.getByText('+5')).toBeInTheDocument();expect(screen.queryByText(/from Alice/)).not.toBeInTheDocument()});
it('creates a receive invoice through the local application',async()=>{const fetch=backend();fetch.mockImplementation(async(url,options)=>{if(String(url).endsWith('/invoice'))return new Response(JSON.stringify({invoice:'lnbcrt-generated'}));if(String(url).endsWith('/session'))return new Response(JSON.stringify({csrf:'test-csrf',busy:false,pending:null,events:[]}));if(String(url).endsWith('/wallet'))return new Response(JSON.stringify({wallet_name:'Bob Wallet',holdings:[{asset,outbound:'105',onchain:'0'}],network:'regtest',sats:null}));return new Response('[]')});render(<App/>);await waitFor(()=>expect(screen.getByText('Bob Wallet')).toBeInTheDocument());fireEvent.click(screen.getByRole('button',{name:'Receive'}));fireEvent.change(screen.getByLabelText('Receiving amount'),{target:{value:'5'}});fireEvent.click(screen.getByRole('button',{name:'Create invoice'}));await waitFor(()=>expect(screen.getByLabelText('Receiving invoice')).toHaveValue('lnbcrt-generated'));expect(fetch).toHaveBeenCalledWith('/api/invoice',expect.objectContaining({body:JSON.stringify({asset_id:'rgb:demo',amount:'5'})}))});

it('shows preparation progress outside Agent while the application is busy',async()=>{
 const fetch=backend();fetch.mockImplementation(async(url)=>{if(String(url).endsWith('/session'))return new Response(JSON.stringify({csrf:'test-csrf',busy:true,pending:null,events:[]}));return new Response('[]')});
 render(<App/>);expect(await screen.findByRole('status')).toHaveTextContent('Working on your request');expect(screen.getByRole('status')).toHaveTextContent('Any required approval will appear here');
 fireEvent.click(screen.getByRole('button',{name:'Agent'}));expect(screen.getByRole('status')).toBeInTheDocument();expect(screen.getByRole('button',{name:'Send message'})).toBeDisabled();
});

it('copies the configured receiving address exactly',async()=>{const writeText=vi.fn().mockResolvedValue(undefined);Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText}});render(<ReceiveAddress address="bob@example.com"/>);fireEvent.click(screen.getByRole('button',{name:'Copy address'}));await waitFor(()=>expect(writeText).toHaveBeenCalledWith('bob@example.com'));expect(screen.getByText('Address copied')).toBeInTheDocument()});
it('does not invent a receiving address when unconfigured',()=>{render(<ReceiveAddress/>);expect(screen.getByText(/No receiving address configured/)).toBeInTheDocument();expect(screen.queryByRole('button',{name:'Copy address'})).not.toBeInTheDocument()});

it('direct Send prepares an invoice without calling the agent',async()=>{
 const fetch=backend();render(<App/>);
 await waitFor(()=>expect(screen.getByTestId('primary-balance')).toHaveTextContent('490'));
 fireEvent.click(screen.getByRole('button',{name:'RGB invoice'}));
 fireEvent.change(screen.getByLabelText('Invoice to pay'),{target:{value:'lnbcrt-direct'}});
 expect(screen.getByText('RGB LIGHTNING ONLY')).toBeInTheDocument();
 expect(screen.getByText(/For BTC, use/)).toBeInTheDocument();
 fireEvent.click(screen.getByRole('button',{name:'Review RGB payment'}));
 await waitFor(()=>expect(fetch).toHaveBeenCalledWith('/api/send/prepare',expect.objectContaining({method:'POST',body:JSON.stringify({invoice:'lnbcrt-direct'})})));
 expect(fetch.mock.calls.some(([url])=>String(url).includes('/agent/message'))).toBe(false);
 expect(fetch.mock.calls.some(([url])=>String(url).includes('/approve'))).toBe(false);
});
it('omits commerce suggestions when commerce is not configured',async()=>{
 backend();render(<App/>);await waitFor(()=>expect(screen.getByTestId('primary-balance')).toHaveTextContent('490'));
 fireEvent.click(screen.getByRole('button',{name:'Agent'}));expect(screen.queryByText('Get the premium report.')).not.toBeInTheDocument();
});
it('shows checking only while activity refreshes',()=>{
 const item={payment_hash:'hash',asset_id:'rgb:demo',amount:'5',timestamp:1,direction:'sent',status:'pending' as Status};
 const {rerender}=render(<ActivityList assets={[asset]} items={[item]} refreshing/>);
 expect(screen.getByText('Checking status…')).toBeInTheDocument();
 rerender(<ActivityList assets={[asset]} items={[item]}/>);
 expect(screen.queryByText('Checking status…')).not.toBeInTheDocument();
});

it('BTC approval names the recipient and sats, never RGB or machine auto-approval',()=>{
 const action=vi.fn();render(<BtcApprovalSheet plan={{plan_id:'btc-bound',recipient:{identifier:'alice@example.com',authoritative_domain:'example.com'},amount_sats:'5',available_sats:'100',payment_hash:'btc-hash',expires_at:9999999999,policy:{decision:'require_approval'}}} busy={false} onAction={action}/>);
 expect(screen.getByRole('dialog')).toHaveAccessibleName('Confirm BTC payment');expect(screen.getByText('5 sats')).toBeInTheDocument();expect(screen.getByText('alice@example.com')).toBeInTheDocument();expect(screen.queryByText(/R402USD/)).not.toBeInTheDocument();expect(action).not.toHaveBeenCalled();fireEvent.click(screen.getByRole('button',{name:'Approve BTC payment'}));expect(action).toHaveBeenCalledWith(true);
});
it('BTC transfers show sats and received direction, separate from machine purchases',()=>{
 render(<ActivityList assets={[]} items={[{kind:'btc_transfer',payment_hash:'btc-hash',asset_id:'BTC',amount:'5',timestamp:1,direction:'received',status:'settled'}]}/>);
 expect(screen.getByText('Received BTC')).toBeInTheDocument();expect(screen.getByText('+5 sats')).toBeInTheDocument();expect(screen.queryByText('Machine service')).not.toBeInTheDocument();
});

it('keeps direct RGB invoices and agent-guided BTC address payments separate',()=>{
 const rgb=vi.fn();const btc=vi.fn();
 render(<PaymentRails wallet={{btc_policy:{max_payment_sats:'5000',max_daily_sats:'10000',human_approval_required:true},btc_outbound_sats:'5000',holdings:[],sats:null,network:'regtest'}} onRgbInvoice={rgb} onAddress={btc}/>);
 expect(screen.getByText('RGB Lightning')).toBeInTheDocument();
 expect(screen.getByText('BTC address payment')).toBeInTheDocument();
 fireEvent.click(screen.getByRole('button',{name:'Paste RGB invoice'}));
 fireEvent.click(screen.getByRole('button',{name:'Pay a recipient address'}));
 expect(rgb).toHaveBeenCalledOnce();expect(btc).toHaveBeenCalledOnce();
});

it('makes a failed BTC payment terminal in activity without offering a retry',()=>{
 render(<ActivityList assets={[]} items={[{kind:'btc_transfer',payment_hash:'btc-failed',asset_id:'BTC',amount:'5',timestamp:1,direction:'sent',status:'failed'}]}/>);
 expect(screen.getByText('Failed at the node. This payment will not retry automatically.')).toBeInTheDocument();
 expect(screen.queryByRole('button',{name:/retry/i})).not.toBeInTheDocument();
});

 it('sends chat on Enter but preserves Shift+Enter and composition',async()=>{
 const fetch=backend();render(<App/>);
 await waitFor(()=>expect(screen.getByTestId('primary-balance')).toHaveTextContent('490'));
 fireEvent.click(screen.getByRole('button',{name:'Agent'}));
 const input=screen.getByLabelText('Message your wallet');
 fireEvent.change(input,{target:{value:'Show my balance'}});
 fireEvent.keyDown(input,{key:'Enter',shiftKey:true});
 fireEvent.keyDown(input,{key:'Enter',isComposing:true});
 expect(fetch.mock.calls.filter(([,options])=>options?.method==='POST')).toHaveLength(0);
 fireEvent.keyDown(input,{key:'Enter'});
 await waitFor(()=>expect(fetch).toHaveBeenCalledWith('/api/agent/message',expect.objectContaining({method:'POST',body:JSON.stringify({message:'Show my balance'})})));
 });
