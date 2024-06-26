# ladder

Command line tool for using clash service without sudo.


`ladder` is a script based on Clash that enables proxy functionality within a session, and it automatically stops when the session is closed. 

**Note 0**: *For the reason that the ladder.sh script uses `export`, users should do `source ladder.sh` for using it.*

**Note 1**: *Due to the design of the Linux system, the automatically configured proxy is only effective within the current session (SSH connection, terminal window, desktop environment). If other windows need to use the proxy opened by another window, please refer to the Linux proxy configuration and configure it according to the output of the ladder command.*

**Note 2**: *The proxy opened by the `ladder` command will automatically exit when the session is closed (window closed, connection disconnected, exit command).*

**Note 3**: *change the value of `CLASH_EXEC` at the beginning of `ladder.sh` before using.*

 - Basic usage:
   ```bash

   # put this in .bashrc or .zshrc
   alias ladder="source /path/to/ladder.sh"

   # set subscription url, run once
   ladder -s https://example.com/subscription_url_of_your_vpn  
   # start service
   ladder  
   # stop service manually 
   # (proceed automatically when session ended)
   ladder -k
   ```
 - `ladder -h` **H**elp.

 - `ladder -s <subscription_url>` **S**et the subscription URL for Clash in the script. This only needs to be run once, unless you need to change the subscription URL.

 - `ladder` Start the proxy and configure the system proxy environment variables in the background silently.

 - `ladder -k` **K**ill current ladder process and clean up proxy related environment variables.

 - `ladder -u` **U**pdate subscription file right away (download from subscription url). 

 - `ladder -v` Start the background proxy and set environment variables and display **v**erbose proxy-related output. It will display the information of nodes you connected to before each connection using the proxy is established. The output content will be appended to the current content (stdout) but will not affect shell operations.

 - `ladder -c` **C**lean up all proxy related environment variables (unset `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`).

 - `ladder -d` **D**isplay the current subscription URL.


