
if [[ "$CLASH_EXEC" == "" ]]; then
    CLASH_EXEC=clash
fi

if [[ "$CLASH_UPDATE_AFTER_SEC" == "" ]]; then 
    CLASH_UPDATE_AFTER_SEC=$(( 3600 * 24 * 1 ))
fi 

if [[ "$CLASH_MAX_PORT" == "" ]]; then
    CLASH_MAX_PORT=50000
fi

if [[ "$CLASH_MIN_PORT" == "" ]]; then
    CLASH_MIN_PORT=30000
fi


CONFIG_DIR=$HOME/.config/clash
SUBSCRIPTION=$CONFIG_DIR/subscription.txt
LOG_FILE=$CONFIG_DIR/clash.log


VERBOSE=false
CLASH_USER_PORT=

_flag=true

function help() {
    echo "-----------------------------------------------------------------"
    echo "USAGE:"
    echo "  \e[32m-s <SUBSCRIPTION URL>\e[0m for \e[4msetting\e[0m subscription url." 
    echo "  \e[32m-k\e[0m \e[4mkill\e[0m current ladder process and unset environment variables."
    echo "  \e[32m-d\e[0m \e[4mdisplay\e[0m current subscription url."  
    echo "  \e[32m-c\e[0m \e[4mclean\e[0m up all proxy environment variables."
    echo "  \e[32m-u\e[0m \e[4mupdate\e[0m subscription right away."
    echo "  \e[32m-p <HTTP_PORT>\e[0m \e[4mports\e[0m to be used for proxy."
    echo "  \e[32m-v\e[0m for \e[4mdetailed\e[0m output of the clash service."
    echo "  \e[32m-h\e[0m for this message."
    echo "-----------------------------------------------------------------"
}


function get_random_port() {
    range=$(( CLASH_MAX_PORT - CLASH_MIN_PORT ))
    echo $(od -An -N2 -i /dev/urandom | \
           awk -v r=$range -v m=$CLASH_MIN_PORT '{print $1 % r + m}')
}


function time_after_update() {
    file=$1
    now=`date +%s`
    last_change=`stat -c %Y $file`
    echo $(( now - last_change ))
}


function download_config() {
    raw_url=`cat $SUBSCRIPTION`
    if [[ "$raw_url" == "" ]]; then
        echo "\e[31mEmpty subscription url found.\e[0m"
        echo "\e[31mPlease use \e[32m-s\e[31m to set url before using.\e[0m"
        return 1
    fi
    encoded_url=$(python3 -c "print('${raw_url}'.replace('\\\\', ''))")
    encoded_url=$(python3 -c "from urllib.parse import quote;print(quote('${encoded_url}', safe=''))")
    echo "\e[34mDownloading new subscription file ... \e[0m"
    curl -k "https://subconverters.com/sub?target=clash&url=${encoded_url}" > $CONFIG_DIR/tmp.yaml
    lines=`wc --lines $CONFIG_DIR/tmp.yaml | awk '{print $1}'`
    if (( lines > 0 )); then 
        mv $CONFIG_DIR/tmp.yaml $CONFIG_DIR/subscription.yaml
        echo "\e[34mDownload complete.\e[0m"
    else
        rm $CONFIG_DIR/tmp.yaml
        echo "\e[33mWarning: subscription file download failed.\e[0m"
        _flag=false
    fi
}


function update_subscription() {
    test -d $CONFIG_DIR || mkdir -p $CONFIG_DIR
    if [[ "$1" == "" ]]; then 
        echo "\e[31mEmpty subscription url\e[0m"
        help
        return 1
    else
        echo "$1" > $SUBSCRIPTION
        download_config 
    fi
}


function update_config() {
    http_port=$1
    socks_port=$2
    controller_port=$3
    if [ -f $CONFIG_DIR/subscription.yaml ]; then
        delta=$(time_after_update $CONFIG_DIR/subscription.yaml)
        if (( delta > CLASH_UPDATE_AFTER_SEC )); then
            download_config
        fi
    else
        download_config
    fi

    sed -e "s/^port:.*/port: $http_port/" \
        -e "s/^socks-port:.*/socks-port: $socks_port/" \
        -e "s/^external-controller:.*/external-controller: localhost:$controller_port/" \
        -e "/^redir-port:.*/d" -e "/^mixed-port:.*/d" \
        $CONFIG_DIR/subscription.yaml > $CONFIG_DIR/config.yaml
}


function start_service() {
    http_port=$1
    socks_port=$2
    ret1=`lsof -i:$http_port`
    ret2=`lsof -i:$socks_port`
    if [[ "$ret1" == "" && "$ret2" == "" ]]; then
        if $VERBOSE; then
            ( $CLASH_EXEC -d $CONFIG_DIR &! echo $! > $CONFIG_DIR/ladder.pid ) 2>&1 | tee $LOG_FILE &! 
            export LADDER_PID=`cat $CONFIG_DIR/ladder.pid`
        else
            $CLASH_EXEC -d $CONFIG_DIR 2>&1 > $LOG_FILE &!
            export LADDER_PID=$!
            echo $LADDER_PID > $CONFIG_DIR/ladder.pid
        fi
    else
        echo "\e[31mPorts unavaliable. Try again.\e[0m"
        return 1
    fi
}



while [[ "$#" -gt 0 ]]; do
    case $1 in
        -s) if [[ "$#" -lt 2 ]]; then
                echo "\e[31mPlease provide subscription url after -s\e[0m"
                help
                return 1
            fi
            update_subscription $2
            return $?
            ;;
        -k) if [[ "$LADDER_PID" == "" ]]; then
                echo "\e[33mladder service is not running.\e[0m"
            else
                kill $LADDER_PID || return 1
                echo "\e[34mladder service $LADDER_PID terminated.\e[0m"         
                unset LADDER_PID
            fi
            unset HTTP_PROXY
            unset HTTPS_PROXY
            unset ALL_PROXY
            unset http_proxy
            unset https_proxy
            unset all_proxy
            return 0
            ;;
        -v) VERBOSE=true
            ;;
        -c) unset HTTP_PROXY
            unset HTTPS_PROXY
            unset ALL_PROXY
            unset http_proxy
            unset https_proxy
            unset all_proxy
            echo "\e[34m*_PROXY variables unset.\e[0m"
            return 0
            ;;
        -d) echo "\e[34mCurrent subscription url:\e[0m\e[4m"
            cat $SUBSCRIPTION
            echo -n "\e[0m"
            return 0
            ;;
        -u) download_config
            if $_flag; then
                return 0
            else
                return 1
            fi
            ;;
        -p) CLASH_USER_PORT=$2
            if [[ "$CLASH_USER_PORT" == "" ]]; then
                echo "\e[31mPlease provide port number after -p\e[0m"
                help
                return 1
            fi
            break
            ;;
        -h) help
            return 0
            ;;
        *) echo "\e[31mUnknown parameter passed: $1\e[0m"
           return 1 
           ;;
    esac
    shift
done



if [ ! -f $SUBSCRIPTION ]; then
    echo "\e[31mSubscription not set yet, please run \e[32m-s <subscription url>\e[31m first.\e[0m"
    return 1
fi

if [[ "$CLASH_USER_PORT" == "" ]]; then
    http_port=`get_random_port`
else
    http_port=$CLASH_USER_PORT
fi
socks_port=$(( http_port + 1 ))
controller_port=$(( http_port + 2 ))
update_config $http_port $socks_port $controller_port
if (( $? != 0 )); then
    echo "\e[31mconfig.yaml update failed.\e[0m"
    return 1
fi

start_service $http_port $socks_port

export HTTP_PROXY=http://127.0.0.1:$http_port
export HTTPS_PROXY=http://127.0.0.1:$http_port
export ALL_PROXY=socks5://127.0.0.1:$socks_port
export http_proxy=http://127.0.0.1:$http_port
export http_proxy=http://127.0.0.1:$http_port
export all_proxy=socks5://127.0.0.1:$socks_port


sleep 2
echo "\e[34m----------------------------------------------------"
echo "> Ladder Service $LADDER_PID started at:\e[32m"
echo "HTTP_PROXY=http://localhost:$http_port"
echo "HTTPS_PROXY=http://localhost:$http_port"
echo "ALL_PROXY=socks5://localhost:$socks_port\e[34m"
echo "External Control at http://localhost:$controller_port"
echo "----------------------------------------------------\e[0m"


function _when_ladder_end_() {
    if [[ "$LADDER_PID" != "" ]]; then
        echo "\e[34mStopping ladder process $LADDER_PID ... \e[0m"
        kill $LADDER_PID
    fi
}

trap '_when_ladder_end_' TERM HUP KILL EXIT 

