import os,io,json,asyncio,logging,traceback
import aiohttp,discord
from datetime import datetime,timezone,timedelta
from dotenv import load_dotenv
from discord.ext import commands
from discord import app_commands
from hash import ScamDetector,DEBUG_USER_ID
from text import TextDetector

load_dotenv()
TOKEN=os.getenv("BOT")
WEBSITE_API=os.getenv("WEBSITE_API")
API_KEY=os.getenv("API_KEY")

with open("config.json","r") as f: config=json.load(f)

detector=ScamDetector("database.aegis")
text_detector=TextDetector("database.aegis")
threats_since_heartbeat=0
messages_since_heartbeat=0
incident_cooldowns={}
COOLDOWN_SECONDS=900
IMAGE_SEMAPHORE=asyncio.Semaphore(2)
MAX_IMAGE_SIZE=3*1024*1024
IMAGE_CAMPAIGN_BONUS=0.03
BETA_APPLICATION_ID=1532972276805537792
LOG_CHANNEL_REFRESH_SECONDS=3600

intents=discord.Intents.default()
intents.message_content=True
intents.guilds=True
intents.members=True
bot=commands.Bot(command_prefix="!",intents=intents)
bot.log_cache={}
bot.update_cache={}
bot.http_session=None

async def send_error_dm(error_text):
    try:
        user=await bot.fetch_user(DEBUG_USER_ID)
        if user: await user.send(f"⚠️ **Bot Error**\n\n```text\n{error_text[:1900]}\n```")
    except Exception: pass

def report_error(error):
    logging.error("Unhandled error:\n%s",traceback.format_exc())
    try: asyncio.create_task(send_error_dm(f"{type(error).__name__}: {error}"))
    except Exception: pass

async def get_http_session():
    if bot.http_session is None or bot.http_session.closed: bot.http_session=aiohttp.ClientSession()
    return bot.http_session

def user_on_cooldown(guild_id,user_id):
    key=(guild_id,user_id); now=datetime.now(timezone.utc).timestamp(); last_time=incident_cooldowns.get(key)
    if last_time is not None and now-last_time<COOLDOWN_SECONDS: return True
    incident_cooldowns[key]=now
    return False

async def find_log_channel_uncached(guild):
    identifier=config["logging_identifier"]
    for channel in guild.text_channels:
        if channel.topic and identifier in channel.topic: return channel
    return None

async def find_log_channel(guild):
    if guild.id in bot.log_cache:
        channel=guild.get_channel(bot.log_cache[guild.id])
        if channel: return channel
    channel=await find_log_channel_uncached(guild)
    if channel: bot.log_cache[guild.id]=channel.id
    return channel

async def find_update_channel_uncached(guild):
    identifier=config["update_identifier"]
    for channel in guild.text_channels:
        if channel.topic and identifier in channel.topic: return channel
    return None

async def find_update_channel(guild):
    if guild.id in bot.update_cache:
        channel=guild.get_channel(bot.update_cache[guild.id])
        if channel: return channel
    channel=await find_update_channel_uncached(guild)
    if channel: bot.update_cache[guild.id]=channel.id
    return channel

async def refresh_log_channels():
    for guild in bot.guilds:
        try:
            old_id=bot.log_cache.get(guild.id); channel=await find_log_channel_uncached(guild)
            if channel:
                bot.log_cache[guild.id]=channel.id
                if old_id!=channel.id:
                    try:
                        embed=discord.Embed(title="📋 Logging Channel Detected",description="This channel has been detected as the bot's logging channel.",color=discord.Color.blue())
                        await channel.send(embed=embed)
                    except discord.HTTPException as e: report_error(e)
        except Exception as e: report_error(e)

async def logging_channel_watcher():
    await bot.wait_until_ready()
    while not bot.is_closed():
        try: await refresh_log_channels()
        except Exception as e: report_error(e)
        await asyncio.sleep(LOG_CHANNEL_REFRESH_SECONDS)

async def download_image(url):
    session=await get_http_session()
    try:
        async with session.get(url,timeout=aiohttp.ClientTimeout(total=15)) as response:
            if response.status!=200: return None
            content_length=response.headers.get("Content-Length")
            if content_length and int(content_length)>MAX_IMAGE_SIZE: return None
            data=await response.read()
            if len(data)>MAX_IMAGE_SIZE: return None
            return data
    except Exception as e:
        report_error(e)
        return None

class FalsePositiveModal(discord.ui.Modal,title="Mark as False Positive"):
    reason=discord.ui.TextInput(label="Why was this a false positive?",placeholder="Explain why you believe the bot incorrectly flagged this content...",style=discord.TextStyle.paragraph,required=True,max_length=1000)

    def __init__(self,data,parent_view):
        super().__init__(); self.data=data; self.parent_view=parent_view

    async def on_submit(self,interaction:discord.Interaction):
        if self.parent_view.submitted:
            await interaction.response.send_message("⚠️ This false positive has already been submitted.",ephemeral=True); return
        self.parent_view.submitted=True
        channel_id=config.get("false_positive_channel_id")
        if not channel_id:
            self.parent_view.submitted=False
            await interaction.response.send_message("❌ `false_positive_channel_id` is not configured in `config.json`.",ephemeral=True); return
        try: channel_id=int(channel_id)
        except (TypeError,ValueError):
            self.parent_view.submitted=False
            await interaction.response.send_message("❌ `false_positive_channel_id` in `config.json` is invalid.",ephemeral=True); return
        channel=bot.get_channel(channel_id)
        if channel is None:
            try: channel=await bot.fetch_channel(channel_id)
            except discord.HTTPException as e:
                self.parent_view.submitted=False; report_error(e)
                await interaction.response.send_message("❌ I couldn't access the configured false-positive channel.",ephemeral=True); return
        data=self.data
        embed=discord.Embed(title="🟡 False Positive Report",description="A scam detection was manually marked as a false positive.",color=discord.Color.gold(),timestamp=datetime.now(timezone.utc))
        embed.add_field(name="User",value=f"{data['user_mention']}\n`{data['user_id']}`",inline=False)
        embed.add_field(name="Server",value=f"{data['guild_name']}\n`{data['guild_id']}`",inline=False)
        embed.add_field(name="Channel",value=data["channel_name"],inline=False)
        embed.add_field(name="Detection Reason",value=data["reason"],inline=False)
        flagged_content=data.get("flagged_content")
        if flagged_content:
            flagged_content=flagged_content.replace("```","'''")
            embed.add_field(name="Flagged Content",value=f"```text\n{flagged_content[:1000]}\n```",inline=False)
        image_result=data.get("image_result")
        if image_result:
            confidence=image_result.get("confidence"); original_confidence=image_result.get("original_confidence")
            if confidence is not None: embed.add_field(name="Confidence",value=f"{confidence:.2%}")
            if original_confidence is not None: embed.add_field(name="Original Confidence",value=f"{original_confidence:.2%}")
            campaign_boost=image_result.get("campaign_boost")
            if campaign_boost: embed.add_field(name="Campaign Bonus",value=f"+{campaign_boost:.2%}")
        embed.add_field(name="Why was this a false positive?",value=self.reason.value[:1000],inline=False)
        embed.add_field(name="Marked By",value=f"{interaction.user.mention}\n`{interaction.user.id}`",inline=False)
        message_url=data.get("message_url")
        if message_url: embed.add_field(name="Original Message",value=message_url,inline=False)
        files=[discord.File(io.BytesIO(image_data),filename=name) for name,image_data in data.get("flagged_images",[])]
        try: await channel.send(embed=embed,files=files)
        except discord.HTTPException as e:
            self.parent_view.submitted=False; report_error(e)
            await interaction.response.send_message("❌ Failed to send the false-positive report.",ephemeral=True); return
        self.parent_view.false_positive_button.disabled=True
        self.parent_view.false_positive_button.label="False Positive Submitted"
        self.parent_view.false_positive_button.emoji="✅"
        try: await interaction.message.edit(view=self.parent_view)
        except discord.HTTPException as e: report_error(e)
        await interaction.response.send_message("✅ False-positive report submitted.",ephemeral=True)

class FalsePositiveView(discord.ui.View):
    def __init__(self,false_positive_data):
        super().__init__(timeout=None); self.false_positive_data=false_positive_data; self.submitted=False

    @discord.ui.button(label="False Positive",emoji="⚠️",style=discord.ButtonStyle.secondary,custom_id="aegis_false_positive")
    async def false_positive_button(self,interaction:discord.Interaction,button:discord.ui.Button):
        if self.submitted:
            await interaction.response.send_message("⚠️ This detection has already been marked as a false positive.",ephemeral=True); return
        modal=FalsePositiveModal(self.false_positive_data,self)
        await interaction.response.send_modal(modal)

async def handle_scam(message,reason,match=None,flagged_images=None,image_result=None):
    global threats_since_heartbeat
    threats_since_heartbeat+=1
    try: await message.delete(); deleted=True
    except discord.HTTPException as e: report_error(e); deleted=False
    if user_on_cooldown(message.guild.id,message.author.id): return
    timeout_status="Timed out successfully"
    try: await message.author.timeout(timedelta(days=config["timeout_days"]),reason="Scam detected")
    except discord.Forbidden: timeout_status="⚠️ Unable to timeout user (missing permissions or role hierarchy)"
    except discord.HTTPException as e: report_error(e); timeout_status="⚠️ Discord API error while timing out"
    if reason=="image_campaign":
        flagged_text="Image attachment matched a known scam campaign."
        if image_result:
            confidence=image_result.get("confidence")
            if confidence is not None: flagged_text+=f"\nConfidence: {confidence:.2%}"
    else: flagged_text=match[:1000] if match else "Unknown"
    dm_text=(f"Hello <@{message.author.id}>,\n\nf" if False else
        f"Hello <@{message.author.id}>,\n\n"
        f"⚠️ **Security Alert**\n\n"
        f"Your account sent content commonly associated with scam campaigns.\n\n"
        f"Your account may have been compromised.\n\n"
        f"Please:\n\n"
        f"🔒 Reset your Discord password\n"
        f"🛡️ Enable Two-Factor Authentication\n"
        f"🦠 Run malware scans\n"
        f"🔗 Review Authorized Apps\n\n\n"
        f"You have been timed out in:\n\n"
        f"**{message.guild.name}**\n\n"
        f"for **7 days**.\n\n\n"
        f"Detected location:\n\n"
        f"<#{message.channel.id}>\n\n\n"
        f"Flagged Content:\n\n"
        f"```text\n"
        f"{flagged_text[:1500]}\n"
        f"```\n\n\n"
        f"If this was a mistake:\n\n"
        f"{config.get('dm_report_server','Not configured')}\n\n\n"
        f"Please include:\n"
        f"- The flagged message\n"
        f"- What you intended to send\n"
        f"- Any useful investigation details\n\n\n"
        f"-# Anti-Scam Protection System").strip()
    dm_files=[discord.File(io.BytesIO(data),filename=name) for name,data in (flagged_images or [])]
    try: await message.author.send(content=dm_text,files=dm_files)
    except discord.HTTPException as e: report_error(e)
    log_files=[discord.File(io.BytesIO(data),filename=name) for name,data in (flagged_images or [])]
    log_channel=await find_log_channel(message.guild)
    if not log_channel: return
    embed=discord.Embed(title="🚨 Scam Detected",description="Possible scam content detected.",color=discord.Color.red(),timestamp=datetime.now(timezone.utc))
    embed.add_field(name="User",value=f"{message.author.mention}\n`{message.author.id}`",inline=False)
    embed.add_field(name="Reason",value=reason,inline=False)
    if match:
        safe_match=match[:1000].replace("```","'''")
        embed.add_field(name="Flagged Content",value=f"```text\n{safe_match}\n```",inline=False)
    elif reason=="image_campaign": embed.add_field(name="Flagged Content",value="Image matched known scam campaign database.",inline=False)
    embed.add_field(name="Channel",value=message.channel.mention)
    embed.add_field(name="Deleted",value=str(deleted))
    embed.add_field(name="Timeout",value=timeout_status,inline=False)
    if image_result:
        confidence=image_result.get("confidence"); original_confidence=image_result.get("original_confidence"); campaign_boost=image_result.get("campaign_boost")
        if confidence is not None: embed.add_field(name="Confidence",value=f"{confidence:.2%}")
        if original_confidence is not None: embed.add_field(name="Original Confidence",value=f"{original_confidence:.2%}")
        if campaign_boost: embed.add_field(name="Campaign Bonus",value=f"+{campaign_boost:.2%}")
    false_positive_data={"user_mention":message.author.mention,"user_id":message.author.id,"guild_name":message.guild.name,"guild_id":message.guild.id,"channel_name":f"{message.channel.mention}\n`{message.channel.id}`","reason":reason,"flagged_content":flagged_text,"message_url":message.jump_url,"flagged_images":flagged_images or [],"image_result":image_result}
    view=FalsePositiveView(false_positive_data)
    try: await log_channel.send(embed=embed,files=log_files,view=view)
    except discord.HTTPException as e: report_error(e)

async def send_heartbeat():
    global threats_since_heartbeat,messages_since_heartbeat
    session=await get_http_session()
    while not bot.is_closed():
        try:
            payload={"servers":len(bot.guilds),"users":sum(guild.member_count or 0 for guild in bot.guilds),"threats":threats_since_heartbeat,"messages":messages_since_heartbeat}
            headers={"Authorization":f"Bearer {API_KEY}","Content-Type":"application/json"}
            async with session.post(WEBSITE_API,json=payload,headers=headers,timeout=aiohttp.ClientTimeout(total=15)) as response:
                if response.status>=400: logging.warning("Heartbeat returned HTTP %s",response.status)
            threats_since_heartbeat=0; messages_since_heartbeat=0
        except Exception as e: report_error(e)
        await asyncio.sleep(300)

@bot.event
async def on_ready():
    print(f"Logged in as {bot.user} (ID: {bot.user.id})")
    await get_http_session()
    try:
        synced=await bot.tree.sync(); print(f"Synced {len(synced)} global slash commands.")
    except Exception as e: report_error(e)
    try:
        test_guild=discord.Object(id=DEBUG_USER_ID); await bot.tree.sync(guild=test_guild)
    except Exception: pass
    if bot.application_id==BETA_APPLICATION_ID: await bot.change_presence(activity=discord.Game("AEGIS BETA"))
    else: await bot.change_presence(activity=discord.Game("protecting servers"))
    if not hasattr(bot,"heartbeat_task") or bot.heartbeat_task.done(): bot.heartbeat_task=asyncio.create_task(send_heartbeat())
    if not hasattr(bot,"logging_watcher_task") or bot.logging_watcher_task.done(): bot.logging_watcher_task=asyncio.create_task(logging_channel_watcher())
    print(f"Connected to {len(bot.guilds)} server(s).")

@bot.event
async def on_error(event,*args,**kwargs):
    error_text=traceback.format_exc()
    logging.error("Error in event %s:\n%s",event,error_text)
    try: await send_error_dm(f"Event: {event}\n\n{error_text}")
    except Exception: pass

@bot.event
async def on_message(message):
    global messages_since_heartbeat
    if message.author.bot or message.guild is None: return
    messages_since_heartbeat+=1
    await bot.process_commands(message)
    text_result=None
    try: text_result=text_detector.scan(message.content)
    except Exception as e: report_error(e)
    if text_result and text_result.get("is_scam"):
        reason=text_result.get("reason","text_scam")
        if reason!="image_campaign":
            await handle_scam(message,reason,text_result.get("match")); return
    image_attachments=[attachment for attachment in message.attachments if attachment.content_type and attachment.content_type.startswith("image/")]
    if not image_attachments: return
    text_campaign_flagged=bool(text_result and text_result.get("is_scam") and text_result.get("reason")=="image_campaign")
    candidates=[]; flagged=[]
    for attachment in image_attachments:
        async with IMAGE_SEMAPHORE:
            image_bytes=await download_image(attachment.url)
            if not image_bytes: continue
            try:
                image_result=await asyncio.wait_for(asyncio.to_thread(detector.scan,image_bytes,message.author.id),timeout=30)
            except asyncio.TimeoutError:
                logging.warning("Image scan timed out for %s",attachment.url); continue
            except Exception as e:
                report_error(e); continue
            if not image_result: continue
            original_confidence=image_result.get("confidence",0); confidence=original_confidence; campaign_boost=0
            if text_campaign_flagged: campaign_boost=IMAGE_CAMPAIGN_BONUS; confidence+=campaign_boost
            image_result["original_confidence"]=original_confidence; image_result["campaign_boost"]=campaign_boost; image_result["confidence"]=confidence
            candidates.append(image_result); flagged.append((attachment.filename,image_bytes))
    if not candidates: return
    highest=max(candidates,key=lambda x:x.get("confidence",0))
    if highest.get("confidence",0)<config["delete_threshold"]: return
    await handle_scam(message,"image_campaign",flagged_images=flagged,image_result=highest)

@bot.tree.command(name="help",description="Show information about the bot.")
async def help_command(interaction:discord.Interaction):
    embed=discord.Embed(title="🛡️ AEGIS",description="AEGIS is an automated scam detection and protection bot.",color=discord.Color.blue())
    embed.add_field(name="Detection",value="Scans messages and images for known scam content.",inline=False)
    embed.add_field(name="Protection",value="Detected scam messages can be removed and users timed out.",inline=False)
    await interaction.response.send_message(embed=embed,ephemeral=True)

@bot.command(name="stats")
async def stats(ctx,server_number:int=None):
    if ctx.author.id!=DEBUG_USER_ID: return
    guilds=sorted(bot.guilds,key=lambda g:g.name.lower())
    if not guilds:
        await ctx.send("I'm not in any servers."); return
    if server_number is not None:
        if server_number<1 or server_number>len(guilds):
            await ctx.send("Invalid server number."); return
        guild=guilds[server_number-1]; me=guild.me or guild.get_member(bot.user.id)
        if me is None:
            await ctx.send("Unable to find my member object in that server."); return
        invite_url=None
        for channel in guild.text_channels:
            permissions=channel.permissions_for(me)
            if not permissions.create_instant_invite: continue
            try:
                invite=await channel.create_invite(max_age=0,max_uses=0,unique=False,reason="Owner stats command"); invite_url=invite.url; break
            except (discord.Forbidden,discord.HTTPException): continue
        if invite_url: server_message=f"**📊 Server Stats:**\n\n🏷️ Name: **{guild.name}**\n👥 Members: **{guild.member_count}**\n\n{invite_url}"
        else: server_message=f"**📊 Server Stats:**\n\n🏷️ Name: **{guild.name}**\n👥 Members: **{guild.member_count}**\n\n❌ Could not create invite."
        await ctx.send(server_message); return
    users=set()
    for guild in guilds:
        for member in guild.members: users.add(member.id)
    stats_message=f"**🤖 Bot Statistics:**\n\n🌐 Servers: **{len(guilds)}**\n👥 Unique Users: **{len(users)}**"
    await ctx.send(stats_message)
    lines=[f"`{index}` {guild.name}" for index,guild in enumerate(guilds,start=1)]
    message="**📋 Server List:**\n\n"+"\n".join(lines)
    if len(message)>2000:
        chunks=[]; current=""
        for line in lines:
            if len(current)+len(line)+1>1900: chunks.append(current); current=line+"\n"
            else: current+=line+"\n"
        if current: chunks.append(current)
        for index,chunk in enumerate(chunks):
            if index==0: await ctx.send("**📋 Server List:**\n\n"+chunk)
            else: await ctx.send(chunk)
    else: await ctx.send(message)

@bot.command(name="announce")
async def announce_command(ctx,*,content=None):
    if ctx.author.id!=DEBUG_USER_ID: return
    if not content:
        await ctx.send("Usage: `!announce <message>`"); return
    for guild in bot.guilds:
        try:
            channel=await find_update_channel(guild)
            if channel: await channel.send(content)
        except Exception as e: report_error(e)

@bot.command(name="set-channel")
async def set_channel_command(ctx,channel:discord.TextChannel=None,channel_type:str=None):
    if not ctx.author.guild_permissions.manage_channels:
        await ctx.send("❌ You need the **Manage Channels** permission to use this command.",delete_after=10); return
    if channel is None:
        await ctx.send("Usage: `!set-channel #channel logging|announce`"); return
    if channel_type not in ("logging","announce"):
        await ctx.send("Channel type must be `logging` or `announce`."); return
    identifier=config["logging_identifier"] if channel_type=="logging" else config["update_identifier"]
    try:
        current_topic=channel.topic or ""
        if identifier not in current_topic:
            if current_topic: current_topic+="\n"
            current_topic+=identifier
            await channel.edit(topic=current_topic)
        await ctx.send(f"✅ {channel.mention} configured as the `{channel_type}` channel.")
    except discord.HTTPException as e:
        report_error(e); await ctx.send("❌ Failed to update the channel.")

@bot.command(name="ping")
async def ping_command(ctx):
    latency=round(bot.latency*1000)
    await ctx.send(f"🏓 Pong! `{latency}ms`")

@bot.event
async def on_guild_remove(guild):
    bot.log_cache.pop(guild.id,None); bot.update_cache.pop(guild.id,None)

async def cleanup():
    if bot.http_session: await bot.http_session.close()

try: bot.run(TOKEN)
finally:
    try: asyncio.run(cleanup())
    except Exception: pass
