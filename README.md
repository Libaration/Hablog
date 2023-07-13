# Hablog
This project is a simple TCP proxy implemented in Rust using the Tokio asynchronous runtime. 
It allows forwarding network traffic between a client and a server. It logs the packets (client only)
This was kinda just an excuse to learn more Rust, but it'd be pretty easy to use as a starting point to add packet headers / packet sending.

## Project Structure
The project consists of the following files:

* `main.rs`: Contains the main entry point of the application. It sets up the network connection and handles user input.
* `lib.rs`: Defines the packet header parsing and handling functions. It uses a HashMap to associate specific header values with corresponding handler functions.
* `connection.rs`: Defines the Connection struct and its methods. It handles incoming connections and manages data forwarding between the server and connected client.
 *Add additional header/packet handling functions in lib.rs and register them in the HashMap.*

### You will need root privileges to run this.
The application checks the /etc/hosts file for a proxy entry for the specified host. If the entry is found, it displays a message. If the entry is not found, it adds a proxy entry for the host in the /etc/hosts file.

# Limited Example


https://github.com/Libaration/Hablog/assets/11550216/e01ad50f-07e5-4dbf-842e-d4c14cbec4cc

# Packet Debugging


https://github.com/Libaration/Hablog/assets/11550216/8865c60e-22a4-442c-9528-f77280a6472a




