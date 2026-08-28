import os
import io
import json
import asyncio
import logging
import traceback

import aiohttp
import discord

from datetime import datetime, timezone, timedelta
from dotenv import load_dotenv
from discord.ext import commands

from hash import ScamDetector, DEBUG_USER_ID
from text import TextDetector


load_dotenv()

TOKEN = os.getenv("BOT")

WEBSITE_API = os.getenv("WEBSITE_API")
API_KEY = os.getenv("API_KEY")


logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s | %(levelname)s | %(message)s"
)

discord.utils.setup_logging(level=logging.INFO)


with open("config.json", "r") as f:
    config = json.load(f)


with open(config["database"], "r") as f:
    database = json.load(f)


detector = ScamDetector(database)

text_detector = TextDetector(
    "antiscam.rules"
)


threats_since_heartbeat = 0
messages_since_heartbeat = 0
incident_cooldowns = {}
COOLDOWN_SECONDS = 900
IMAGE_SEMAPHORE = asyncio.Semaphore(2)
MAX_IMAGE_SIZE = 3 * 1024 * 1024
BETA_APPLICATION_ID = 1532972276805537792
LOG_CHANNEL_REFRESH_SECONDS = 3600

intents = discord.Intents.default()
intents.message_content = True
intents.guilds = True
intents.members = True


bot = commands.Bot(
    command_prefix="!",
    intents=intents
)


bot.log_cache = {}
bot.http_session = None


async def send_error_dm(error_text):

    try:
        user = bot.get_user(DEBUG_USER_ID)

        if user is None:
            user = await bot.fetch_user(DEBUG_USER_ID)

        await user.send(
            f"🚨 AntiScam Bot Error\n\n```py\n{error_text[:1900]}\n```"
        )

    except Exception:
        logging.exception(
            "Failed to send error DM"
        )


def report_error(error):

    text = "".join(
        traceback.format_exception(
            type(error),
            error,
            error.__traceback__
        )
    )

    logging.error(text)

    asyncio.create_task(
        send_error_dm(text)
    )


async def get_http_session():

    if (
        bot.http_session is None
        or bot.http_session.closed
    ):

        bot.http_session = aiohttp.ClientSession(
            timeout=aiohttp.ClientTimeout(
                total=20
            )
        )

        logging.info(
            "Created HTTP session"
        )

    return bot.http_session


def user_on_cooldown(guild_id, user_id):

    now = datetime.now(timezone.utc)

    expired = [
        key
        for key, timestamp in incident_cooldowns.items()
        if (
            now - timestamp
        ).total_seconds() > COOLDOWN_SECONDS * 2
    ]

    for key in expired:
        del incident_cooldowns[key]

    key = (
        guild_id,
        user_id
    )

    timestamp = incident_cooldowns.get(
        key
    )

    if timestamp is None:

        incident_cooldowns[key] = now

        return False


    if (
        now - timestamp
    ).total_seconds() >= COOLDOWN_SECONDS:

        incident_cooldowns[key] = now

        return False


    return True


async def find_log_channel_uncached(guild):

    for channel in guild.text_channels:

        topic = channel.topic

        if (
            topic
            and config["logging_identifier"] in topic
        ):

            return channel

    return None

async def find_log_channel(guild):

    cached = bot.log_cache.get(
        guild.id
    )

    if cached is not None:
        return cached


    channel = await find_log_channel_uncached(
        guild
    )

    if channel is not None:

        bot.log_cache[guild.id] = channel

        return channel


    bot.log_cache[guild.id] = False

    return None


async def refresh_log_channels():

    for guild in bot.guilds:

        previous = bot.log_cache.get(
            guild.id
        )

        channel = await find_log_channel_uncached(
            guild
        )

        if channel is None:

            bot.log_cache[guild.id] = False

            continue


        bot.log_cache[guild.id] = channel


        channel_changed = (
            previous is not None
            and previous is not False
            and previous.id != channel.id
        )

        newly_detected = (
            previous is None
            or previous is False
        )


        if not (
            newly_detected
            or channel_changed
        ):

            continue


        embed = discord.Embed(
            title="📋 Logging Channel Detected",
            description=(
                "Aegis Sentinel has detected this channel "
                "as the server's logging channel.\n\n"
                "Scam detection logs will be sent here."
            ),
            color=discord.Color.green()
        )

        embed.add_field(
            name="Server",
            value=guild.name,
            inline=False
        )

        embed.add_field(
            name="Channel",
            value=channel.mention,
            inline=False
        )


        try:

            await channel.send(
                embed=embed
            )

            logging.info(
                f"Logging channel detected for "
                f"{guild.name}: #{channel.name}"
            )

        except discord.HTTPException as e:

            report_error(e)


async def logging_channel_watcher():

    while True:

        try:

            await refresh_log_channels()

        except Exception as e:

            report_error(e)


        await asyncio.sleep(
            LOG_CHANNEL_REFRESH_SECONDS
        )


async def download_image(url):

    try:

        session = await get_http_session()

        async with session.get(url) as response:

            if response.status != 200:

                logging.warning(
                    f"Image download failed HTTP {response.status}"
                )

                return None


            if response.content_length:

                if response.content_length > MAX_IMAGE_SIZE:

                    logging.warning(
                        "Image too large"
                    )

                    return None


            data = bytearray()


            async for chunk in response.content.iter_chunked(8192):

                data.extend(chunk)

                if len(data) > MAX_IMAGE_SIZE:

                    logging.warning(
                        "Image exceeded limit"
                    )

                    return None


            return bytes(data)


    except Exception as e:

        report_error(e)

        return None


async def handle_scam(
    message,
    reason,
    match=None,
    files=None,
    image_result=None
):

    global threats_since_heartbeat


    threats_since_heartbeat += 1

    try:

        await message.delete()

        deleted = True

    except discord.HTTPException as e:

        report_error(e)

        deleted = False


    if user_on_cooldown(
        message.guild.id,
        message.author.id
    ):

        return


    timeout_status = "Timed out successfully"


    try:

        await message.author.timeout(
            timedelta(
                days=config["timeout_days"]
            ),
            reason="Scam detected"
        )

    except discord.Forbidden:

        timeout_status = (
            "⚠️ Unable to timeout user "
            "(missing permissions or role hierarchy)"
        )

    except discord.HTTPException as e:

        report_error(e)

        timeout_status = (
            "⚠️ Discord API error while timing out"
        )


    if files is None:

        files = []


    if reason == "image_campaign":

        flagged_text = (
            "Image attachment matched a known scam campaign."
        )

        if image_result:

            flagged_text += (
                f"\nConfidence: {image_result['confidence']:.2%}"
            )

    else:

        flagged_text = (
            match[:1000]
            if match
            else "Unknown"
        )


    dm_text = f"""
Hello {message.author.mention},

⚠️ **Security Alert**

Your account sent content commonly associated with scam campaigns.

Your account may have been compromised.

Please:

🔒 Reset your Discord password

🛡️ Enable Two-Factor Authentication

🦠 Run malware scans

🔗 Review Authorized Apps


You have been timed out in:

**{message.guild.name}**

for **{config["timeout_days"]} days**.


Detected location:

{message.channel.mention}


Flagged Content:

```text
{flagged_text}```


If this was a mistake:

{config["dm_report_server"]}


Please include:
- The flagged message
- What you intended to send
- Any useful investigation details


-# Anti-Scam Protection System
"""


    try:

        await message.author.send(
            content=dm_text,
            files=files
        )

    except discord.HTTPException as e:

        report_error(e)


    log_channel = await find_log_channel(
        message.guild
    )


    if log_channel:

        embed = discord.Embed(
            title="🚨 Scam Detected",
            description="Possible scam content detected.",
            color=discord.Color.red()
        )

        embed.add_field(
            name="User",
            value=(
                f"{message.author.mention}\n"
                f"`{message.author.id}`"
            ),
            inline=False
        )

        embed.add_field(
            name="Reason",
            value=reason,
            inline=False
        )

        if match:

            embed.add_field(
                name="Flagged Content",
                value=f"```text\n{match[:1000]}\n```",
                inline=False
            )

        elif reason == "image_campaign":

            embed.add_field(
                name="Flagged Content",
                value="Image matched known scam campaign database.",
                inline=False
            )

        embed.add_field(
            name="Channel",
            value=message.channel.mention
        )

        embed.add_field(
            name="Deleted",
            value=str(deleted)
        )

        embed.add_field(
            name="Timeout",
            value=timeout_status,
            inline=False
        )


        try:

            await log_channel.send(
                embed=embed,
                files=files
            )

        except discord.HTTPException as e:

            report_error(e)
async def send_heartbeat():

    global threats_since_heartbeat
    global messages_since_heartbeat

    while True:

        try:

            session = await get_http_session()


            users = set()


            for guild in bot.guilds:

                for member in guild.members:

                    users.add(member.id)


            threats = threats_since_heartbeat


            payload = {

                "servers": len(bot.guilds),

                "users": len(users),

                "threatsBlocked": threats,

                "messagesScanned": messages_since_heartbeat

            }


            headers = {

                "Authorization":
                f"Bearer {API_KEY}",

                "Content-Type":
                "application/json"

            }


            async with session.post(
                WEBSITE_API,
                json=payload,
                headers=headers
            ) as response:


                if response.status == 200:

                    threats_since_heartbeat = 0
                    messages_since_heartbeat = 0

                    logging.info(
                        f"Heartbeat sent | Threats: {threats}"
                    )


                else:

                    logging.warning(
                        f"Heartbeat failed: {response.status}"
                    )


        except Exception as e:

            report_error(e)


        await asyncio.sleep(300)


@bot.event
async def on_ready():

    await get_http_session()


    if bot.application_id == BETA_APPLICATION_ID:

        mode = "BETA"

        await bot.change_presence(
            activity=discord.Game(
                name="BETA TESTING | Aegis Sentinel"
            )
        )


    else:

        mode = "PRODUCTION"

        await bot.change_presence(
            activity=discord.Game(
                name="Protecting servers 🛡️"
            )
        )


    if not hasattr(bot, "heartbeat_task"):

        bot.heartbeat_task = asyncio.create_task(
            send_heartbeat()
        )


    if not hasattr(bot, "logging_channel_task"):

        bot.logging_channel_task = asyncio.create_task(
            logging_channel_watcher()
        )
    print(
        f"""
=========================
Aegis Sentinel {mode}

Logged in:
{bot.user}

Application ID:
{bot.application_id}

Servers:
{len(bot.guilds)}
=========================
"""
    )


@bot.event
async def on_error(event, *args, **kwargs):

    error = traceback.format_exc()

    logging.error(
        f"Unhandled event error in {event}:\n{error}"
    )


    await send_error_dm(
        f"Event: {event}\n\n{error}"
    )


@bot.event
async def on_message(message):
    global messages_since_heartbeat


    if message.author.bot:

        return


    if message.guild is None:

        return
    messages_since_heartbeat += 1

    await bot.process_commands(
        message
    )


    try:

        text_result = text_detector.scan(
            message.content
        )

    except Exception as e:

        report_error(e)

        text_result = {
            "detected": False
        }


    if text_result.get("detected"):

        await handle_scam(
            message,
            text_result.get(
                "reason",
                "text_scam"
            ),
            text_result.get(
                "match"
            )
        )

        # Intentionally stop here.
        # If the message already contains malicious text,
        # the message is deleted and the image scan is skipped.

        return


    flagged = []
    highest = None


    async with IMAGE_SEMAPHORE:

        for attachment in message.attachments:

            try:

                content_type = attachment.content_type


                if not content_type:

                    logging.info(
                        f"No content type: {attachment.filename}"
                    )

                    continue


                if not content_type.startswith("image"):

                    continue


                logging.info(
                    f"Downloading image: {attachment.filename}"
                )


                image_bytes = await download_image(
                    attachment.url
                )


                if not image_bytes:

                    continue


                logging.info(
                    "Starting image scan"
                )


                try:

                    result = await asyncio.wait_for(

                        asyncio.to_thread(

                            detector.scan,

                            image_bytes,

                            message.author.id

                        ),

                        timeout=30

                    )


                except asyncio.TimeoutError:

                    raise RuntimeError(
                        "Image scan timed out after 30 seconds"
                    )


                logging.info(
                    "Finished image scan"
                )


                if result:

                    flagged.append(
                        (
                            attachment.filename,
                            image_bytes
                        )
                    )


                    if (
                        highest is None
                        or result["confidence"]
                        > highest["confidence"]
                    ):

                        highest = result


            except Exception as e:

                report_error(e)


    if highest is None:

        return


    confidence = highest["confidence"]


    if confidence < config["delete_threshold"]:

        return


    files = [

        discord.File(
            io.BytesIO(data),
            filename=name
        )

        for name, data in flagged

    ]


    await handle_scam(

        message,

        "image_campaign",

        files=files,

        image_result=highest

    )

@bot.command(name="stats")
async def stats(ctx, server_number: int = None):

    if ctx.author.id != DEBUG_USER_ID:

        return


    guilds = sorted(
        bot.guilds,
        key=lambda g: g.name.lower()
    )


    if not guilds:

        await ctx.send(
            "I'm not in any servers."
        )

        return


    if server_number is not None:

        if (
            server_number < 1
            or server_number > len(guilds)
        ):

            await ctx.send(
                "Invalid server number."
            )

            return


        guild = guilds[
            server_number - 1
        ]


        me = (
            guild.me
            or guild.get_member(
                bot.user.id
            )
        )


        if me is None:

            await ctx.send(
                "Unable to find my member object in that server."
            )

            return


        invite_url = None


        for channel in guild.text_channels:

            permissions = channel.permissions_for(
                me
            )


            if not permissions.create_instant_invite:

                continue


            try:

                invite = await channel.create_invite(
                    max_age=0,
                    max_uses=0,
                    unique=False,
                    reason="Owner stats command"
                )


                invite_url = invite.url

                break


            except (
                discord.Forbidden,
                discord.HTTPException
            ):

                continue


        if invite_url:

            server_message = (
                "**📊 Server Stats:**\n\n"
                f"🏷️ Name: **{guild.name}**\n"
                f"👥 Members: **{guild.member_count}**\n\n"
                f"{invite_url}"
            )


        else:

            server_message = (
                "**📊 Server Stats:**\n\n"
                f"🏷️ Name: **{guild.name}**\n"
                f"👥 Members: **{guild.member_count}**\n\n"
                "❌ Could not create invite."
            )


        await ctx.send(
            server_message
        )

        return


    users = set()


    for guild in guilds:

        for member in guild.members:

            users.add(
                member.id
            )


    stats_message = (
        "**🤖 Bot Statistics:**\n\n"
        f"🌐 Servers: **{len(guilds)}**\n"
        f"👥 Unique Users: **{len(users)}**"
    )


    await ctx.send(
        stats_message
    )


    lines = []


    for index, guild in enumerate(
        guilds,
        start=1
    ):

        lines.append(
            f"`{index}` {guild.name}"
        )


    message = (
        "**📋 Server List:**\n\n"
        +
        "\n".join(lines)
    )


    if len(message) > 2000:

        chunks = []

        current = ""


        for line in lines:

            if (
                len(current)
                +
                len(line)
                +
                1
                >
                1900
            ):

                chunks.append(
                    current
                )

                current = (
                    line
                    +
                    "\n"
                )


            else:

                current += (
                    line
                    +
                    "\n"
                )


        if current:

            chunks.append(
                current
            )


        for index, chunk in enumerate(chunks):

            if index == 0:

                await ctx.send(
                    "**📋 Server List:**\n\n"
                    + chunk
                )


            else:

                await ctx.send(
                    chunk
                )


    else:

        await ctx.send(
            message
        )


@bot.command()
async def ping(ctx):

    print(
        "ping command called"
    )


    await ctx.send(
        "# Pong"
    )


@bot.event
async def on_guild_remove(guild):

    bot.log_cache.pop(
        guild.id,
        None
    )


bot.run(
    TOKEN
)
